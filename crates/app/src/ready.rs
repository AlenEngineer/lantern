//! The ready check: what in a plan would go wrong on stage, found before
//! the service. Each step is looked at the way the stage will load it
//! ([`stage::needs`]): something it can't show, a missing file, a YouTube
//! download not done or failed (with yt-dlp chosen), an image link not
//! downloaded yet. A YouTube video in YouTube's own player has nothing to
//! get ready: it plays from YouTube, with the internet, as it goes on.

use lantern_engine::{Action, Plan, Resource, StepId};

use crate::config::YoutubeMode;
use crate::stage::{self, Needs};
use crate::youtube::{Download, YoutubeVideo};

/// How bad an issue is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// It may still sort itself out: a download under way, an image that
    /// will download when shown.
    Waiting,
    /// It won't show as things stand.
    Problem,
}

/// One step that isn't ready, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub step: StepId,
    pub level: Level,
    pub what: String,
}

/// Every issue in `plan`, in plan order. `youtube` is how YouTube videos
/// play; `download` is one's download state, with yt-dlp; `cached` says
/// whether an image link is downloaded.
pub fn check(
    plan: &Plan,
    youtube: YoutubeMode,
    download: impl Fn(&YoutubeVideo) -> Option<Download>,
    cached: impl Fn(&str) -> bool,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    for step in plan.steps() {
        let Action::Show { resource } = &step.action else {
            continue;
        };
        let issue = |level, what: String| Issue {
            step: step.id,
            level,
            what,
        };
        // The sound that plays with it is a file to have, too.
        if let Some(sound) = &step.sound {
            match stage::needs(&sound.resource) {
                Err(why) => issues.push(issue(Level::Problem, format!("its sound: {why}"))),
                Ok(Needs::File(path)) if !path.is_file() => {
                    issues.push(issue(Level::Problem, "its sound's file is missing".into()))
                }
                Ok(_) => {}
            }
        }
        if let Some((level, what)) = issue_of(resource, youtube, &download, &cached) {
            issues.push(issue(level, what));
        }
    }
    issues
}

/// What would go wrong showing `resource`, and how bad it is, as
/// [`check`] finds it for a step; also a search result's warning.
pub fn issue_of(
    resource: &Resource,
    youtube: YoutubeMode,
    download: impl Fn(&YoutubeVideo) -> Option<Download>,
    cached: impl Fn(&str) -> bool,
) -> Option<(Level, String)> {
    match stage::needs(resource) {
        Err(why) => Some((Level::Problem, why)),
        Ok(Needs::Nothing) => None,
        Ok(Needs::File(path)) if !path.is_file() => Some((Level::Problem, "file missing".into())),
        Ok(Needs::File(_)) => None,
        // In YouTube's own player first: nothing is downloaded.
        Ok(Needs::Youtube(_))
            if matches!(youtube, YoutubeMode::Official | YoutubeMode::OfficialFirst) =>
        {
            None
        }
        Ok(Needs::Youtube(video)) => match download(&video) {
            // A live video plays as it comes: nothing to wait for.
            Some(Download::Ready(_) | Download::Live) => None,
            // YouTube's player plays it instead, from the internet.
            Some(Download::Failed(why)) => Some((
                Level::Waiting,
                format!("download failed ({why}): plays in YouTube's player, needs the internet"),
            )),
            Some(state @ (Download::Queued | Download::Running(_))) => {
                Some((Level::Waiting, state.to_string()))
            }
            None => Some((Level::Waiting, "not downloaded yet".into())),
        },
        Ok(Needs::Images(urls)) if !urls.iter().any(|url| cached(url)) => Some((
            Level::Waiting,
            "not downloaded yet: needs the internet".into(),
        )),
        Ok(Needs::Images(_)) => None,
    }
}

/// Where what `resource` shows comes from when it is shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Reach {
    /// This computer: a file, typed text, a download that is done.
    #[default]
    Here,
    /// The internet, until it is downloaded.
    Online,
    /// The internet, all the while it shows: a video that is live.
    Stream,
}

/// Where `resource` comes from when shown, `download` and `cached` as
/// [`check`] takes them.
pub fn reach(
    resource: &Resource,
    download: impl Fn(&YoutubeVideo) -> Option<Download>,
    cached: impl Fn(&str) -> bool,
) -> Reach {
    match stage::needs(resource) {
        Ok(Needs::Youtube(video)) => match download(&video) {
            Some(Download::Ready(_)) => Reach::Here,
            Some(Download::Live) => Reach::Stream,
            _ => Reach::Online,
        },
        Ok(Needs::Images(urls)) if !urls.iter().any(|url| cached(url)) => Reach::Online,
        _ => Reach::Here,
    }
}

/// One line for the whole plan, "ready", "2 not ready yet", "1 problem",
/// and the worst level (`None` when ready). `None` for a plan that shows
/// nothing, so there's nothing to check.
pub fn summary(plan: &Plan, issues: &[Issue]) -> Option<(String, Option<Level>)> {
    if !plan
        .steps()
        .iter()
        .any(|s| matches!(s.action, Action::Show { .. }))
    {
        return None;
    }
    let problems = issues.iter().filter(|i| i.level == Level::Problem).count();
    let waiting = issues.len() - problems;
    Some(match (problems, waiting) {
        (0, 0) => ("ready".into(), None),
        (0, n) => (format!("{n} not ready yet"), Some(Level::Waiting)),
        (1, _) => ("1 problem".into(), Some(Level::Problem)),
        (n, _) => (format!("{n} problems"), Some(Level::Problem)),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use lantern_engine::{Kind, Location, Resource, ResourceId, SourceId};

    use super::*;

    fn file(path: &Path, kind: Kind) -> Resource {
        let source = SourceId::new("test").unwrap();
        let id = ResourceId::new(&source, &path.to_string_lossy());
        Resource::new(id, "x", kind, Location::File(path.to_path_buf()))
    }

    fn youtube(id: &str) -> Resource {
        let source = SourceId::new("link").unwrap();
        lantern_source_link::youtube_video(&source, id)
    }

    #[test]
    fn missing_files_unshowable_kinds_and_downloads_are_found() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("hymn.png");
        std::fs::write(&present, "").unwrap();
        let mut plan = Plan::new("Sunday");
        plan.show(file(&present, Kind::Image));
        let missing = plan.show(file(&dir.path().join("gone.mp4"), Kind::Video));
        let docx = plan.show(file(&present, Kind::Document));
        let done = plan.show(youtube("aaaaaaaaaaa"));
        let going = plan.show(youtube("bbbbbbbbbbb"));
        let failed = plan.show(youtube("ccccccccccc"));
        let download = |video: &YoutubeVideo| match video.id.as_str() {
            "aaaaaaaaaaa" => Some(Download::Ready("x.mp4".into())),
            "bbbbbbbbbbb" => Some(Download::Running(42.0)),
            _ => Some(Download::Failed("Video unavailable".into())),
        };
        let issues = check(&plan, YoutubeMode::YtDlpFirst, download, |_| false);
        let found: Vec<(StepId, Level)> = issues.iter().map(|i| (i.step, i.level)).collect();
        assert_eq!(
            found,
            [
                (missing, Level::Problem),
                (docx, Level::Problem),
                (going, Level::Waiting),
                (failed, Level::Waiting),
            ]
        );
        assert!(!found.iter().any(|(step, _)| *step == done));
        assert_eq!(issues[2].what, "downloading 42%");
        assert!(
            issues[3].what.contains("YouTube's player"),
            "{}",
            issues[3].what
        );
        assert_eq!(
            summary(&plan, &issues),
            Some(("2 problems".into(), Some(Level::Problem)))
        );

        // In YouTube's own player, the videos have nothing to get ready.
        let issues = check(&plan, YoutubeMode::Official, download, |_| false);
        let steps: Vec<StepId> = issues.iter().map(|i| i.step).collect();
        assert_eq!(steps, [missing, docx]);
        let issues = check(&plan, YoutubeMode::OfficialFirst, download, |_| false);
        assert_eq!(issues.len(), 2);
    }

    #[test]
    fn one_thing_says_what_would_go_wrong_and_where_it_comes_from() {
        let dir = tempfile::tempdir().unwrap();
        let gone = file(&dir.path().join("gone.mp4"), Kind::Video);
        let none = |_: &YoutubeVideo| None;
        assert_eq!(
            issue_of(&gone, YoutubeMode::YtDlpFirst, none, |_| false),
            Some((Level::Problem, "file missing".into()))
        );
        assert_eq!(reach(&gone, none, |_| false), Reach::Here);

        let video = youtube("aaaaaaaaaaa");
        let state = |state: Download| move |_: &YoutubeVideo| Some(state.clone());
        assert_eq!(reach(&video, none, |_| false), Reach::Online);
        assert_eq!(
            issue_of(&video, YoutubeMode::YtDlpFirst, none, |_| false),
            Some((Level::Waiting, "not downloaded yet".into()))
        );
        assert_eq!(
            issue_of(&video, YoutubeMode::Official, none, |_| false),
            None,
            "in YouTube's player, nothing waits"
        );
        assert_eq!(
            reach(&video, state(Download::Running(40.0)), |_| false),
            Reach::Online
        );
        assert_eq!(
            reach(&video, state(Download::Live), |_| false),
            Reach::Stream
        );
        assert_eq!(
            reach(&video, state(Download::Ready("x.mp4".into())), |_| false),
            Reach::Here
        );
    }

    #[test]
    fn a_plan_with_nothing_wrong_is_ready_and_an_empty_one_says_nothing() {
        let mut plan = Plan::new("Sunday");
        assert_eq!(summary(&plan, &[]), None);
        plan.push("Prayer", Action::Blank);
        assert_eq!(
            summary(&plan, &[]),
            None,
            "nothing to show, nothing to check"
        );
        let source = SourceId::new("bible").unwrap();
        let verse = Resource::new(
            ResourceId::new(&source, "John.3.16"),
            "Juan 3:16",
            Kind::Text,
            Location::Inline("Porque…".into()),
        );
        plan.show(verse);
        let issues = check(&plan, YoutubeMode::Official, |_| None, |_| false);
        assert!(issues.is_empty());
        assert_eq!(summary(&plan, &issues), Some(("ready".into(), None)));
    }
}
