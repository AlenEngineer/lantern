use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use lantern_resource::Resource;
use serde::{Deserialize, Serialize};

use crate::scene::{Media, MediaReport, Playback, Scene, SceneId, Timed};

/// Everything a window needs to draw the stage from scratch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StageState {
    /// On stage now.
    pub program: Option<Scene>,
    /// Lined up by the operator; never shown to the audience.
    pub preview: Option<Scene>,
    /// When `true`, the stage is black regardless of `program`.
    pub blackout: bool,
}

impl StageState {
    /// Applies one event to this state.
    ///
    /// The [`Showman`] changes its own state only through this function. So
    /// a window that starts from a [`StageFeed`]'s `state` and applies every
    /// event after it, in order, holds exactly the engine's state.
    ///
    /// Returns whether anything changed. A `Seek` on a timed program always
    /// counts, because the stage must act on it even when the target equals
    /// the last known position.
    pub fn apply(&mut self, event: &StageEvent) -> bool {
        match event {
            StageEvent::Program { scene } => set_if_different(&mut self.program, scene),
            StageEvent::Preview { scene } => set_if_different(&mut self.preview, scene),
            StageEvent::Frame { frame } => match self.program_media() {
                Some(Media::Frames { frame: current, .. }) => set_if_different(current, frame),
                _ => false,
            },
            StageEvent::Playback { playback } => match self.program_media() {
                Some(Media::Timed(timed)) => set_if_different(&mut timed.playback, playback),
                _ => false,
            },
            StageEvent::Seek { position } => match self.program_media() {
                Some(Media::Timed(timed)) => {
                    timed.seek(*position);
                    true
                }
                _ => false,
            },
            StageEvent::Blackout { on } => set_if_different(&mut self.blackout, on),
            StageEvent::Media { scene, media } => self
                .scene_mut(*scene)
                .is_some_and(|scene| set_if_different(&mut scene.media, media)),
        }
    }

    /// The scene with this id, in program or preview. A window whose media
    /// finished loading asks this to learn whether the scene is still on
    /// stage and what the operator did meanwhile.
    pub fn scene(&self, id: SceneId) -> Option<&Scene> {
        [&self.program, &self.preview]
            .into_iter()
            .flatten()
            .find(|scene| scene.id == id)
    }

    fn scene_mut(&mut self, id: SceneId) -> Option<&mut Scene> {
        [&mut self.program, &mut self.preview]
            .into_iter()
            .flatten()
            .find(|scene| scene.id == id)
    }

    fn program_media(&mut self) -> Option<&mut Media> {
        self.program.as_mut().map(|scene| &mut scene.media)
    }
}

fn set_if_different<T: PartialEq + Clone>(slot: &mut T, value: &T) -> bool {
    if slot == value {
        return false;
    }
    *slot = value.clone();
    true
}

/// A change to the stage. Serializable so it can cross a process boundary
/// (a webview stage) unchanged.
///
/// A stage window applies every event to its mirror with
/// [`StageState::apply`], then acts on three: `Program` (load the new
/// scene), `Playback` (play or pause the player) and `Seek` (jump).
/// Everything else only needs a redraw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum StageEvent {
    /// Something else is on stage (or nothing). Always a new `SceneId`.
    Program {
        /// The new scene.
        scene: Option<Scene>,
    },
    /// The preview changed. Only the dashboard cares.
    Preview {
        /// The new preview.
        scene: Option<Scene>,
    },
    /// The program moved to another frame.
    Frame {
        /// The new frame.
        frame: usize,
    },
    /// The operator played or paused the program.
    Playback {
        /// The new state.
        playback: Playback,
    },
    /// Jump the program's timeline. Clamped to the duration when known.
    Seek {
        /// From the start.
        #[serde(rename = "position_ms", with = "crate::scene::millis")]
        position: Duration,
    },
    /// Blackout toggled.
    Blackout {
        /// Whether the stage is now black.
        on: bool,
    },
    /// A renderer's report changed how a scene is presented. Carries the
    /// resulting `Media`, not the report, so mirrors apply the outcome
    /// instead of re-running the report policy. It names the scene because
    /// it can be about the preview.
    Media {
        /// Which scene.
        scene: SceneId,
        /// Its media now.
        media: Media,
    },
}

/// A window's starting point and every change after it.
///
/// Made under one lock, so `events` holds exactly the changes after `state`,
/// with none missing and none already in `state`.
#[derive(Debug)]
pub struct StageFeed {
    /// The stage when the feed was made.
    pub state: StageState,
    /// Every later change, in order. Apply each one to `state` with
    /// [`StageState::apply`]. Dropping the receiver unsubscribes.
    pub events: Receiver<StageEvent>,
}

/// Owns the stage state and tells subscribers about every change.
///
/// Not synchronized; the engine wraps it in a mutex. Each method decides
/// which events an action means, and `emit` is the only writer of the state.
#[derive(Debug, Default)]
pub struct Showman {
    state: StageState,
    subscribers: Vec<Sender<StageEvent>>,
    last_scene: u64,
}

impl Showman {
    /// Empty stage, no subscribers.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current state.
    pub fn state(&self) -> &StageState {
        &self.state
    }

    /// The scene on stage, if any.
    pub fn program(&self) -> Option<&Scene> {
        self.state.program.as_ref()
    }

    /// The current state and every change after it. The only way to
    /// subscribe, so a window can't take them at different moments.
    pub fn subscribe(&mut self) -> StageFeed {
        let (tx, rx) = mpsc::channel();
        self.subscribers.push(tx);
        StageFeed {
            state: self.state.clone(),
            events: rx,
        }
    }

    /// Puts a resource on stage as a new scene.
    pub fn show(&mut self, resource: Arc<Resource>) {
        let scene = Scene::new(self.next_id(), resource);
        self.emit(StageEvent::Program { scene: Some(scene) });
    }

    /// Lines a resource up in preview as a new scene.
    pub fn preview(&mut self, resource: Arc<Resource>) {
        let scene = Scene::new(self.next_id(), resource);
        self.emit(StageEvent::Preview { scene: Some(scene) });
    }

    /// Empties the preview.
    pub fn clear_preview(&mut self) {
        self.emit(StageEvent::Preview { scene: None });
    }

    /// Moves the preview to program under a fresh id, keeping what renderers
    /// already reported about it. Returns `false` if the preview was empty.
    pub fn take(&mut self) -> bool {
        let Some(scene) = self.state.preview.clone() else {
            return false;
        };
        let id = self.next_id();
        self.emit(StageEvent::Preview { scene: None });
        self.emit(StageEvent::Program {
            scene: Some(Scene { id, ..scene }),
        });
        true
    }

    /// Empties the stage.
    pub fn clear(&mut self) {
        self.emit(StageEvent::Program { scene: None });
    }

    /// Turns blackout on or off.
    pub fn set_blackout(&mut self, on: bool) {
        self.emit(StageEvent::Blackout { on });
    }

    /// Flips blackout. Returns the new value.
    pub fn toggle_blackout(&mut self) -> bool {
        self.set_blackout(!self.state.blackout);
        self.state.blackout
    }

    /// Advances the program one frame. Returns `false` at the last frame,
    /// and for scenes without frames.
    pub fn next_frame(&mut self) -> bool {
        match self.program() {
            Some(scene) if scene.has_next_frame() => self.goto_frame(scene.frame() + 1),
            _ => false,
        }
    }

    /// Goes back one frame. Returns `false` at the first frame.
    pub fn prev_frame(&mut self) -> bool {
        match self.program().map(Scene::frame) {
            Some(frame) if frame > 0 => self.goto_frame(frame - 1),
            _ => false,
        }
    }

    /// Jumps to a frame. Returns `false` if it doesn't exist.
    pub fn goto_frame(&mut self, frame: usize) -> bool {
        match self.program().map(|scene| &scene.media) {
            Some(Media::Frames { count, .. }) if frame < count.map_or(1, |n| n.get()) => {
                self.emit(StageEvent::Frame { frame });
                true
            }
            _ => false,
        }
    }

    /// Plays the program. From `Ended`, replays from the start. Returns
    /// `false` if the program has no timeline.
    pub fn play(&mut self) -> bool {
        let Some(timed) = self.program_timed() else {
            return false;
        };
        if timed.playback == Playback::Ended {
            self.emit(StageEvent::Seek {
                position: Duration::ZERO,
            });
        }
        self.emit(StageEvent::Playback {
            playback: Playback::Playing,
        });
        true
    }

    /// Pauses the program. Changes nothing once ended. Returns `false` if
    /// the program has no timeline.
    pub fn pause(&mut self) -> bool {
        let Some(timed) = self.program_timed() else {
            return false;
        };
        if timed.playback == Playback::Playing {
            self.emit(StageEvent::Playback {
                playback: Playback::Paused,
            });
        }
        true
    }

    /// The play/pause button. Pauses if playing, otherwise
    /// [`play`](Self::play). Returns `false` if the program has no timeline.
    pub fn toggle_playback(&mut self) -> bool {
        match self.program_timed() {
            Some(timed) if timed.playback == Playback::Playing => self.pause(),
            _ => self.play(),
        }
    }

    /// Jumps the program's timeline, clamped to the duration when known.
    /// Always emits, even to the same position. Returns `false` if the
    /// program has no timeline.
    pub fn seek(&mut self, position: Duration) -> bool {
        let Some(timed) = self.program_timed() else {
            return false;
        };
        let position = timed.duration.map_or(position, |end| position.min(end));
        self.emit(StageEvent::Seek { position });
        true
    }

    /// Applies a renderer's report to the scene it names. Returns `false` if
    /// that scene has left the stage or the report doesn't apply to it. A
    /// report that applies but changes nothing returns `true` and emits
    /// nothing.
    ///
    /// Never changes which scene is in a slot, only a scene's `media`.
    pub fn report(&mut self, scene: SceneId, report: MediaReport) -> bool {
        let Some(current) = self.state.scene(scene) else {
            return false;
        };
        let Some(media) = current.media.after(&current.resource, &report) else {
            return false;
        };
        self.emit(StageEvent::Media { scene, media });
        true
    }

    fn next_id(&mut self) -> SceneId {
        self.last_scene += 1;
        SceneId::new(self.last_scene)
    }

    fn program_timed(&self) -> Option<Timed> {
        self.program().and_then(Scene::timed).copied()
    }

    /// The only writer of the state. Subscribers hear only about events that
    /// changed something, which keeps every mirror equal to this state.
    fn emit(&mut self, event: StageEvent) {
        if self.state.apply(&event) {
            self.subscribers.retain(|tx| tx.send(event.clone()).is_ok());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use lantern_resource::{Kind, Location, ResourceId, SourceId};

    use super::*;

    fn resource(kind: Kind, location: Location) -> Arc<Resource> {
        let source = SourceId::new("test").unwrap();
        Arc::new(Resource::new(
            ResourceId::new(&source, "x"),
            "X",
            kind,
            location,
        ))
    }

    fn hymn() -> Arc<Resource> {
        resource(
            Kind::Text,
            Location::Inline("Stanza 1\n\nChorus\n\nStanza 2".into()),
        )
    }

    fn video() -> Arc<Resource> {
        resource(Kind::Video, Location::File("/v.mp4".into()))
    }

    fn document() -> Arc<Resource> {
        resource(Kind::Document, Location::File("/a.pdf".into()))
    }

    fn pages(n: usize) -> MediaReport {
        MediaReport::Frames(NonZeroUsize::new(n).unwrap())
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn program_id(stage: &Showman) -> SceneId {
        stage.program().unwrap().id
    }

    fn timed(stage: &Showman) -> Timed {
        *stage.program().unwrap().timed().unwrap()
    }

    #[test]
    fn frames_stop_at_the_edges() {
        let mut stage = Showman::new();
        stage.show(hymn());
        assert_eq!(stage.program().unwrap().text().as_deref(), Some("Stanza 1"));
        assert!(!stage.prev_frame());
        assert!(stage.next_frame());
        assert!(stage.next_frame());
        assert_eq!(stage.program().unwrap().text().as_deref(), Some("Stanza 2"));
        assert!(!stage.next_frame());
        assert!(!stage.goto_frame(3));
        assert!(stage.goto_frame(0));
    }

    #[test]
    fn documents_learn_their_frame_count() {
        let mut stage = Showman::new();
        stage.show(document());
        let id = program_id(&stage);
        assert!(!stage.next_frame());
        assert!(stage.report(id, pages(3)));
        assert!(stage.next_frame());
        assert!(stage.next_frame());
        assert!(stage.report(id, pages(2)));
        assert_eq!(stage.program().unwrap().frame(), 1);
    }

    #[test]
    fn a_page_count_clamps_the_frame() {
        let mut stage = Showman::new();
        stage.show(document());
        let id = program_id(&stage);
        stage.report(id, pages(5));
        stage.goto_frame(4);
        let feed = stage.subscribe();
        assert!(stage.report(id, pages(2)));
        let events: Vec<_> = feed.events.try_iter().collect();
        assert_eq!(
            events,
            [StageEvent::Media {
                scene: id,
                media: Media::Frames {
                    frame: 1,
                    count: NonZeroUsize::new(2),
                },
            }]
        );
    }

    #[test]
    fn take_moves_preview_to_program() {
        let mut stage = Showman::new();
        assert!(!stage.take());
        stage.preview(video());
        assert!(stage.program().is_none());
        assert!(stage.take());
        assert!(stage.state().preview.is_none());
        assert_eq!(stage.program().unwrap().resource.kind, Kind::Video);
    }

    #[test]
    fn take_gives_a_fresh_id_and_keeps_what_was_reported() {
        let mut stage = Showman::new();
        stage.preview(document());
        let preview = stage.state().preview.as_ref().unwrap().id;
        stage.report(preview, pages(4));
        assert!(stage.take());
        let program = stage.program().unwrap();
        assert_ne!(program.id, preview);
        assert!(program.id > preview);
        assert!(program.has_next_frame());
        assert!(!stage.report(preview, pages(1)));
    }

    #[test]
    fn playback_only_for_timed_media() {
        let mut stage = Showman::new();
        stage.show(hymn());
        assert!(!stage.play());
        assert!(!stage.seek(secs(1)));
        stage.show(video());
        assert_eq!(timed(&stage).playback, Playback::Paused);
        assert!(stage.play());
        assert_eq!(timed(&stage).playback, Playback::Playing);
        assert!(stage.toggle_playback());
        assert_eq!(timed(&stage).playback, Playback::Paused);
    }

    #[test]
    fn a_report_for_a_scene_that_left_is_ignored() {
        let mut stage = Showman::new();
        stage.show(document());
        let gone = program_id(&stage);
        stage.show(document());
        let feed = stage.subscribe();
        assert!(!stage.report(gone, pages(3)));
        assert!(!stage.next_frame());
        assert_eq!(feed.events.try_iter().count(), 0);
    }

    #[test]
    fn a_failed_report_takes_away_the_timeline() {
        let mut stage = Showman::new();
        stage.show(resource(
            Kind::Video,
            Location::Url("https://youtu.be/x".into()),
        ));
        let id = program_id(&stage);
        assert!(stage.report(
            id,
            MediaReport::Failed {
                reason: "can't play links".into()
            }
        ));
        assert!(!stage.play());
        assert!(!stage.seek(secs(5)));
        assert!(!stage.next_frame());

        // A retry that worked brings the timeline back.
        assert!(stage.report(id, MediaReport::Timeline { duration: None }));
        assert!(stage.play());
    }

    #[test]
    fn a_frames_report_on_a_timed_scene_is_ignored() {
        let mut stage = Showman::new();
        stage.show(video());
        let id = program_id(&stage);
        assert!(!stage.report(id, pages(1)));
        assert!(stage.play());
    }

    #[test]
    fn ended_is_sticky_until_play_or_seek() {
        let mut stage = Showman::new();
        stage.show(video());
        let id = program_id(&stage);
        stage.report(
            id,
            MediaReport::Timeline {
                duration: Some(secs(30)),
            },
        );
        stage.play();
        assert!(stage.report(id, MediaReport::Ended));
        assert_eq!(timed(&stage).playback, Playback::Ended);
        assert_eq!(timed(&stage).position, secs(30));

        assert!(!stage.report(id, MediaReport::Position(secs(29))));
        assert!(stage.pause());
        assert_eq!(timed(&stage).playback, Playback::Ended);

        assert!(stage.seek(secs(60)));
        assert_eq!(timed(&stage).playback, Playback::Paused);
        assert_eq!(timed(&stage).position, secs(30));

        stage.report(id, MediaReport::Ended);
        assert!(stage.play());
        assert_eq!(timed(&stage).playback, Playback::Playing);
        assert_eq!(timed(&stage).position, Duration::ZERO);
    }

    #[test]
    fn repeated_reports_emit_nothing() {
        let mut stage = Showman::new();
        stage.show(video());
        let id = program_id(&stage);
        let feed = stage.subscribe();
        let duration = MediaReport::Timeline {
            duration: Some(secs(30)),
        };
        assert!(stage.report(id, duration.clone()));
        assert!(stage.report(id, duration));
        assert!(stage.report(id, MediaReport::Position(secs(3))));
        assert!(stage.report(id, MediaReport::Position(secs(3))));
        assert_eq!(feed.events.try_iter().count(), 2);
    }

    #[test]
    fn inline_text_takes_the_slide_count_the_stage_needed_but_never_plays() {
        let mut stage = Showman::new();
        stage.show(hymn());
        let id = program_id(&stage);
        assert!(!stage.report(id, MediaReport::Timeline { duration: None }));
        // The stage split the text into more slides than its stanzas.
        assert!(stage.report(id, pages(5)));
        for _ in 0..4 {
            assert!(stage.next_frame());
        }
        assert!(!stage.next_frame());
    }

    #[test]
    fn subscribers_see_events_and_can_leave() {
        let mut stage = Showman::new();
        let feed = stage.subscribe();
        let gone = stage.subscribe();
        drop(gone);

        stage.show(hymn());
        stage.next_frame();
        stage.toggle_blackout();
        stage.clear();
        stage.clear(); // no-op, no event

        let events: Vec<_> = feed.events.try_iter().collect();
        assert!(matches!(events[0], StageEvent::Program { scene: Some(_) }));
        assert_eq!(events[1], StageEvent::Frame { frame: 1 });
        assert_eq!(events[2], StageEvent::Blackout { on: true });
        assert_eq!(events[3], StageEvent::Program { scene: None });
        assert_eq!(events.len(), 4);
        assert_eq!(stage.subscribers.len(), 1);
    }

    #[test]
    fn a_mirror_built_from_a_feed_equals_the_stage() {
        let mut stage = Showman::new();
        stage.show(hymn());
        stage.next_frame();
        stage.preview(document());
        let StageFeed { mut state, events } = stage.subscribe();

        let preview = stage.state().preview.as_ref().unwrap().id;
        stage.report(preview, pages(3));
        stage.take();
        stage.next_frame();
        stage.report(preview, pages(1));
        let doc = program_id(&stage);
        stage.report(doc, pages(1));
        stage.toggle_blackout();
        stage.show(video());
        let clip = program_id(&stage);
        stage.report(
            clip,
            MediaReport::Timeline {
                duration: Some(secs(10)),
            },
        );
        stage.play();
        stage.report(clip, MediaReport::Position(secs(4)));
        stage.seek(secs(4));
        stage.pause();
        stage.report(clip, MediaReport::Ended);
        stage.report(clip, MediaReport::Position(secs(2)));
        stage.toggle_playback();
        stage.preview(hymn());
        stage.report(doc, pages(9));
        stage.set_blackout(false);
        stage.clear_preview();
        stage.clear();
        stage.show(document());
        stage.report(
            program_id(&stage),
            MediaReport::Failed {
                reason: "unreadable".into(),
            },
        );

        let mut applied = 0;
        for event in events.try_iter() {
            assert!(state.apply(&event), "every event changes the mirror");
            applied += 1;
        }
        assert!(applied > 15);
        assert_eq!(&state, stage.state());
    }

    #[test]
    fn events_and_reports_serialize_with_millis() {
        let json = serde_json::to_string(&StageEvent::Frame { frame: 2 }).unwrap();
        assert_eq!(json, r#"{"event":"frame","frame":2}"#);

        let seek = StageEvent::Seek {
            position: Duration::from_millis(1500),
        };
        let json = serde_json::to_string(&seek).unwrap();
        assert_eq!(json, r#"{"event":"seek","position_ms":1500}"#);
        assert_eq!(serde_json::from_str::<StageEvent>(&json).unwrap(), seek);

        let media = StageEvent::Media {
            scene: SceneId::new(7),
            media: Media::Timed(Timed {
                playback: Playback::Playing,
                position: Duration::from_millis(250),
                duration: None,
            }),
        };
        let json = serde_json::to_string(&media).unwrap();
        assert_eq!(
            json,
            r#"{"event":"media","scene":7,"media":{"type":"timed","playback":"playing","position_ms":250,"duration_ms":null}}"#
        );
        assert_eq!(serde_json::from_str::<StageEvent>(&json).unwrap(), media);

        let report = MediaReport::Position(secs(2));
        assert_eq!(
            serde_json::to_string(&report).unwrap(),
            r#"{"position":2000}"#
        );
        assert_eq!(serde_json::to_string(&pages(3)).unwrap(), r#"{"frames":3}"#);
        assert!(serde_json::from_str::<MediaReport>(r#"{"frames":0}"#).is_err());
    }
}
