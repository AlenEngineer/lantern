//! The ready check: what in a plan would go wrong on stage, found before
//! the service. Each step is looked at the way the stage will load it
//! ([`stage::needs`]): something it can't show, a missing file, a YouTube
//! download not done or failed, an image link not downloaded yet.

use lantern_engine::{Action, Plan, StepId};

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

/// Every issue in `plan`, in plan order. `download` is a YouTube video's
/// download state; `cached` says whether an image link is downloaded.
pub fn check(
    plan: &Plan,
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
        match stage::needs(resource) {
            Err(why) => issues.push(issue(Level::Problem, why)),
            Ok(Needs::Nothing) => {}
            Ok(Needs::File(path)) if !path.is_file() => {
                issues.push(issue(Level::Problem, "file missing".into()))
            }
            Ok(Needs::File(_)) => {}
            Ok(Needs::Youtube(video)) => match download(&video) {
                Some(Download::Ready(_)) => {}
                Some(Download::Failed(why)) => {
                    issues.push(issue(Level::Problem, format!("download failed: {why}")))
                }
                Some(state @ (Download::Queued | Download::Running(_))) => {
                    issues.push(issue(Level::Waiting, state.to_string()))
                }
                None => issues.push(issue(Level::Waiting, "not downloaded yet".into())),
            },
            Ok(Needs::Images(urls)) if !urls.iter().any(|url| cached(url)) => issues.push(issue(
                Level::Waiting,
                "not downloaded yet: needs the internet".into(),
            )),
            Ok(Needs::Images(_)) => {}
        }
    }
    issues
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
        let issues = check(&plan, download, |_| false);
        let found: Vec<(StepId, Level)> = issues.iter().map(|i| (i.step, i.level)).collect();
        assert_eq!(
            found,
            [
                (missing, Level::Problem),
                (docx, Level::Problem),
                (going, Level::Waiting),
                (failed, Level::Problem),
            ]
        );
        assert!(!found.iter().any(|(step, _)| *step == done));
        assert_eq!(issues[2].what, "downloading 42%");
        assert_eq!(
            summary(&plan, &issues),
            Some(("3 problems".into(), Some(Level::Problem)))
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
        let issues = check(&plan, |_| None, |_| false);
        assert!(issues.is_empty());
        assert_eq!(summary(&plan, &issues), Some(("ready".into(), None)));
    }
}
