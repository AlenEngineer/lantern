//! YouTube videos' titles in plans: filled in for a pasted link, and kept
//! no older than YouTube allows.
//!
//! A pasted link is only an id, titled "YouTube video <id>" until its
//! title is asked for. YouTube's terms say a title that came from its API
//! is fetched again, or dropped, within 30 days ([`KEEP`]). So a video in
//! the open plan whose title is missing or older than that is asked for
//! again through YouTube's oEmbed address (no key needed), and when Lantern
//! starts, so are the old ones in saved plans. One that can't be fetched
//! when it's too old goes back to "YouTube video <id>".
//!
//! A step keeps the label the operator gave it. One still labelled with
//! the video's old title takes the new one.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lantern_engine::{Action, Plan, Resource};
use lantern_source_youtube::FETCHED;
use serde::Deserialize;

use crate::youtube::YoutubeVideo;

/// How long a title from YouTube is kept before it is fetched again.
pub const KEEP: Duration = Duration::from_secs(30 * 24 * 60 * 60);

const OEMBED: &str = "https://www.youtube.com/oembed";

/// A video's title and channel, as YouTube says them now.
#[derive(Debug, Clone, PartialEq)]
pub struct Title {
    pub title: String,
    pub channel: Option<String>,
}

/// Seconds since 1970.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The title YouTube gives `id`, through oEmbed. Blocks.
pub fn fetch(id: &str) -> Result<Title, String> {
    let url = format!("https://www.youtube.com/watch?v={id}");
    let body = ureq::get(OEMBED)
        .query("url", &url)
        .query("format", "json")
        .call()
        .and_then(|mut r| r.body_mut().read_to_string())
        .map_err(|err| format!("YouTube's title for {id}: {err}"))?;
    parse(&body).ok_or_else(|| format!("YouTube's title for {id}: an odd answer"))
}

/// oEmbed's answer: `title` and `author_name`.
pub fn parse(json: &str) -> Option<Title> {
    #[derive(Deserialize)]
    struct Answer {
        title: Option<String>,
        author_name: Option<String>,
    }
    let answer: Answer = serde_json::from_str(json).ok()?;
    let title = answer.title.filter(|t| !t.trim().is_empty())?;
    Some(Title {
        title,
        channel: answer.author_name.filter(|c| !c.trim().is_empty()),
    })
}

/// "YouTube video <id>": what a video is called before its title is known.
fn placeholder(id: &str) -> String {
    format!("YouTube video {id}")
}

/// When `resource`'s title was fetched, if it came from YouTube.
fn fetched(resource: &Resource) -> Option<u64> {
    resource.meta.get(FETCHED).and_then(|at| at.parse().ok())
}

/// Whether a title fetched at `at` is too old to keep at `now`.
fn too_old(at: u64, now: u64) -> bool {
    now.saturating_sub(at) >= KEEP.as_secs()
}

/// The videos in `plan` whose titles are to be asked for at `now`: those
/// too old, and, with `missing`, those never asked for.
pub fn due(plan: &Plan, now: u64, missing: bool) -> Vec<String> {
    let mut plan = plan.clone();
    let mut ids = Vec::new();
    plan.each_resource_mut(|resource| {
        let Some(video) = YoutubeVideo::of(resource) else {
            return;
        };
        let wanted = match fetched(resource) {
            Some(at) => too_old(at, now),
            None => missing && resource.title == placeholder(&video.id),
        };
        if wanted && !ids.contains(&video.id) {
            ids.push(video.id);
        }
    });
    ids
}

/// Puts what was asked for `id` into `plan`: the title YouTube gave, or,
/// when it couldn't be had, the placeholder in place of a title too old to
/// keep. Returns whether anything changed.
pub fn apply(plan: &mut Plan, id: &str, answer: &Result<Title, String>, now: u64) -> bool {
    // A step labelled with the video's title follows it; one the operator
    // named stays as named.
    let mut relabel: Vec<(lantern_engine::StepId, String)> = Vec::new();
    for step in plan.steps() {
        if let Action::Show { resource } = &step.action
            && YoutubeVideo::of(resource).is_some_and(|v| v.id == id)
            && step.label == resource.title
        {
            relabel.push((step.id, step.label.clone()));
        }
    }
    let mut changed = false;
    plan.each_resource_mut(|resource| {
        if !YoutubeVideo::of(resource).is_some_and(|v| v.id == id) {
            return;
        }
        let before = resource.clone();
        match answer {
            Ok(title) => {
                resource.title = title.title.clone();
                resource.subtitle = Some(subtitle(title.channel.as_deref(), resource));
                resource.meta.insert(FETCHED.into(), now.to_string());
            }
            Err(_) => match fetched(resource) {
                Some(at) if too_old(at, now) => {
                    resource.title = placeholder(id);
                    resource.subtitle = Some(subtitle(None, resource));
                    resource.meta.remove(FETCHED);
                }
                _ => {}
            },
        }
        changed |= *resource != before;
    });
    for (step, old) in relabel {
        let title = plan
            .steps()
            .iter()
            .find(|s| s.id == step)
            .and_then(|s| match &s.action {
                Action::Show { resource } => Some(resource.title.clone()),
                _ => None,
            });
        if let (Some(step), Some(title)) = (plan.get_mut(step), title)
            && title != old
        {
            step.label = title;
            changed = true;
        }
    }
    changed
}

/// The channel and length, as a search result says them, or "YouTube".
fn subtitle(channel: Option<&str>, resource: &Resource) -> String {
    let length = resource
        .meta
        .get(Resource::DURATION)
        .and_then(|s| s.parse::<u64>().ok())
        .map(clock);
    let parts: Vec<String> = channel
        .map(str::to_string)
        .into_iter()
        .chain(length)
        .collect();
    if parts.is_empty() || channel.is_none() {
        // Without a channel there's nothing from YouTube to say but that.
        return "YouTube".into();
    }
    parts.join(" · ")
}

/// `3:05`, or `1:02:03` past an hour.
fn clock(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Titles being asked for on a thread of their own, and the answers.
#[derive(Default)]
pub struct Titles {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Asked for this session, so each is asked once.
    asked: HashSet<String>,
    waiting: Vec<String>,
    answers: Vec<(String, Result<Title, String>)>,
    running: bool,
}

impl Titles {
    /// Asks for each of `ids` not asked for yet.
    pub fn ask(self: &Arc<Self>, ids: Vec<String>) {
        let mut state = self.lock();
        for id in ids {
            if state.asked.insert(id.clone()) {
                state.waiting.push(id);
            }
        }
        if state.running || state.waiting.is_empty() {
            return;
        }
        state.running = true;
        drop(state);
        let titles = Arc::clone(self);
        std::thread::spawn(move || {
            loop {
                let Some(id) = ({
                    let mut state = titles.lock();
                    let next = state.waiting.pop();
                    if next.is_none() {
                        state.running = false;
                    }
                    next
                }) else {
                    return;
                };
                let answer = fetch(&id);
                titles.lock().answers.push((id, answer));
            }
        });
    }

    /// The answers come since last asked.
    pub fn take(&self) -> Vec<(String, Result<Title, String>)> {
        std::mem::take(&mut self.lock().answers)
    }

    /// Whether any answer is still to come or to be taken.
    pub fn busy(&self) -> bool {
        let state = self.lock();
        state.running || !state.answers.is_empty()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Fetches again the titles too old to keep in every saved plan but
/// `open`, and writes back those that changed. Blocks: for a thread of its
/// own as Lantern starts.
pub fn refresh_saved(store: &crate::plans::Store, open: Option<&std::path::Path>) {
    let Ok(entries) = store.list() else {
        return;
    };
    let now = now();
    for entry in entries {
        if open == Some(entry.path.as_path()) {
            continue;
        }
        let Ok(summary) = entry.summary else {
            continue;
        };
        let mut plan = summary.plan;
        let ids = due(&plan, now, false);
        if ids.is_empty() {
            continue;
        }
        let mut changed = false;
        for id in ids {
            changed |= apply(&mut plan, &id, &fetch(&id), now);
        }
        if changed && let Err(err) = store.write(&entry.path, &plan) {
            crate::log(format!("titles: {}: {err}", entry.path.display()));
        }
    }
}

#[cfg(test)]
mod tests {
    use lantern_engine::{SourceId, StepSound};
    use lantern_source_link::youtube_video;

    use super::*;

    const DAY: u64 = 24 * 60 * 60;

    fn video(id: &str) -> Resource {
        youtube_video(&SourceId::new("link").unwrap(), id)
    }

    fn from_search(id: &str, title: &str, at: u64) -> Resource {
        let mut resource = video(id);
        resource.title = title.into();
        resource.subtitle = Some("Old channel · 6:22".into());
        resource
            .with_meta(Resource::DURATION, "382")
            .with_meta(FETCHED, at.to_string())
    }

    fn answer(title: &str) -> Result<Title, String> {
        Ok(Title {
            title: title.into(),
            channel: Some("Channel".into()),
        })
    }

    #[test]
    fn oembed_answers_read_as_title_and_channel() {
        let json = r#"{"title":"Sublime Gracia","author_name":"Himnos","type":"video","html":"<iframe></iframe>"}"#;
        assert_eq!(
            parse(json),
            Some(Title {
                title: "Sublime Gracia".into(),
                channel: Some("Himnos".into())
            })
        );
        assert_eq!(parse(r#"{"title":"  "}"#), None);
        assert_eq!(parse("Not Found"), None);
    }

    #[test]
    fn pasted_links_and_old_titles_are_due_and_fresh_ones_are_not() {
        let now = 100 * DAY;
        let mut plan = Plan::default();
        plan.push(
            "pasted",
            Action::Show {
                resource: video("pastedpaste"),
            },
        );
        plan.push(
            "fresh",
            Action::Show {
                resource: from_search("freshfreshf", "Fresh", now - 29 * DAY),
            },
        );
        let old = plan.push(
            "old",
            Action::Show {
                resource: from_search("oldoldoldol", "Old", now - 30 * DAY),
            },
        );
        plan.get_mut(old).unwrap().sound = Some(StepSound::new(from_search("soundsounds", "S", 0)));
        assert_eq!(
            due(&plan, now, true),
            ["pastedpaste", "oldoldoldol", "soundsounds"]
        );
        // Saved plans only need what is too old: a pasted link's id isn't
        // anything from YouTube.
        assert_eq!(due(&plan, now, false), ["oldoldoldol", "soundsounds"]);
    }

    #[test]
    fn a_fetched_title_replaces_the_old_one_and_a_step_named_after_it() {
        let now = 100 * DAY;
        let mut plan = Plan::default();
        let same = plan.push(
            "Old",
            Action::Show {
                resource: from_search("abcdefghijk", "Old", 0),
            },
        );
        let named = plan.push(
            "Opening video",
            Action::Show {
                resource: from_search("abcdefghijk", "Old", 0),
            },
        );
        plan.standby.push(from_search("abcdefghijk", "Old", 0));

        assert!(apply(&mut plan, "abcdefghijk", &answer("New"), now));
        let shown = |id| match &plan.steps().iter().find(|s| s.id == id).unwrap().action {
            Action::Show { resource } => resource.clone(),
            _ => unreachable!(),
        };
        assert_eq!(shown(same).title, "New");
        assert_eq!(shown(same).subtitle.as_deref(), Some("Channel · 6:22"));
        assert_eq!(shown(same).meta[FETCHED], now.to_string());
        assert_eq!(plan.steps()[0].label, "New", "it was named after the video");
        assert_eq!(
            plan.steps().iter().find(|s| s.id == named).unwrap().label,
            "Opening video",
            "the operator named this one"
        );
        assert_eq!(plan.standby[0].title, "New");
        assert!(
            !apply(&mut plan, "abcdefghijk", &answer("New"), now),
            "nothing more to change"
        );
    }

    #[test]
    fn a_title_too_old_to_keep_goes_when_it_cant_be_fetched() {
        let now = 100 * DAY;
        let mut plan = Plan::default();
        plan.push(
            "Old",
            Action::Show {
                resource: from_search("oldoldoldol", "Old", 0),
            },
        );
        plan.push(
            "Fresh",
            Action::Show {
                resource: from_search("freshfreshf", "Fresh", now - DAY),
            },
        );
        let offline = Err("offline".to_string());
        assert!(apply(&mut plan, "oldoldoldol", &offline, now));
        assert!(
            !apply(&mut plan, "freshfreshf", &offline, now),
            "a fresh one stays"
        );
        let Action::Show { resource } = &plan.steps()[0].action else {
            unreachable!()
        };
        assert_eq!(resource.title, "YouTube video oldoldoldol");
        assert_eq!(resource.subtitle.as_deref(), Some("YouTube"));
        assert!(!resource.meta.contains_key(FETCHED));
        assert_eq!(plan.steps()[0].label, "YouTube video oldoldoldol");
    }
}
