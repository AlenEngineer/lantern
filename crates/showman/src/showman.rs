use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use chorquest_resource::Resource;
use serde::{Deserialize, Serialize};

use crate::{Playback, Scene};

/// Everything a stage window needs to draw itself from scratch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StageState {
    /// On stage now.
    pub program: Option<Scene>,
    /// Lined up by the operator; never shown to the audience.
    pub preview: Option<Scene>,
    /// When `true`, the stage is black regardless of `program`.
    pub blackout: bool,
}

/// A change to the stage. Events are serializable so they can cross a
/// process boundary (e.g. a webview) unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum StageEvent {
    /// Something else is on stage (or nothing).
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
    /// Play or pause.
    Playback {
        /// The new state.
        playback: Playback,
    },
    /// Jump to a point in the timeline.
    Seek {
        /// Milliseconds from the start.
        position_ms: u64,
    },
    /// Blackout toggled.
    Blackout {
        /// Whether the stage is now black.
        on: bool,
    },
}

/// Owns the stage state and tells subscribers about every change.
///
/// Not synchronized; the engine wraps it in a mutex.
#[derive(Debug, Default)]
pub struct Showman {
    state: StageState,
    subscribers: Vec<Sender<StageEvent>>,
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

    /// Receives every future event. Dropping the receiver unsubscribes.
    pub fn subscribe(&mut self) -> Receiver<StageEvent> {
        let (tx, rx) = mpsc::channel();
        self.subscribers.push(tx);
        rx
    }

    /// Puts a resource on stage directly.
    pub fn show(&mut self, resource: Arc<Resource>) {
        self.set_program(Some(Scene::new(resource)));
    }

    /// Lines a resource up in preview.
    pub fn preview(&mut self, resource: Arc<Resource>) {
        self.state.preview = Some(Scene::new(resource));
        self.emit(StageEvent::Preview {
            scene: self.state.preview.clone(),
        });
    }

    /// Empties the preview.
    pub fn clear_preview(&mut self) {
        if self.state.preview.take().is_some() {
            self.emit(StageEvent::Preview { scene: None });
        }
    }

    /// Moves the preview to program. Returns what went on stage.
    pub fn take(&mut self) -> Option<Arc<Resource>> {
        let scene = self.state.preview.take()?;
        let resource = Arc::clone(&scene.resource);
        self.emit(StageEvent::Preview { scene: None });
        self.set_program(Some(scene));
        Some(resource)
    }

    /// Empties the stage.
    pub fn clear(&mut self) {
        if self.state.program.is_some() {
            self.set_program(None);
        }
    }

    /// Turns blackout on or off.
    pub fn set_blackout(&mut self, on: bool) {
        if self.state.blackout != on {
            self.state.blackout = on;
            self.emit(StageEvent::Blackout { on });
        }
    }

    /// Flips blackout. Returns the new value.
    pub fn toggle_blackout(&mut self) -> bool {
        self.set_blackout(!self.state.blackout);
        self.state.blackout
    }

    /// Advances the program one frame. Returns `false` at the last frame.
    pub fn next_frame(&mut self) -> bool {
        match self.state.program.as_ref() {
            Some(scene) if scene.has_next_frame() => self.goto_frame(scene.frame + 1),
            _ => false,
        }
    }

    /// Goes back one frame. Returns `false` at the first frame.
    pub fn prev_frame(&mut self) -> bool {
        match self.state.program.as_ref() {
            Some(scene) if scene.frame > 0 => self.goto_frame(scene.frame - 1),
            _ => false,
        }
    }

    /// Jumps to a frame. Returns `false` if it doesn't exist.
    pub fn goto_frame(&mut self, frame: usize) -> bool {
        let Some(scene) = self.state.program.as_mut() else {
            return false;
        };
        if frame >= scene.frame_count.unwrap_or(1) {
            return false;
        }
        if scene.frame != frame {
            scene.frame = frame;
            self.emit(StageEvent::Frame { frame });
        }
        true
    }

    /// Called by the stage window once it knows how many pages or slides the
    /// program has. Doesn't re-announce the scene; only clamps the frame if needed.
    pub fn set_frame_count(&mut self, count: usize) {
        let Some(scene) = self.state.program.as_mut() else {
            return;
        };
        let count = count.max(1);
        scene.frame_count = Some(count);
        if scene.frame >= count {
            scene.frame = count - 1;
            let frame = scene.frame;
            self.emit(StageEvent::Frame { frame });
        }
    }

    /// Starts playback. Returns `false` if the program isn't timed media.
    pub fn play(&mut self) -> bool {
        self.set_playback(Playback::Playing)
    }

    /// Pauses playback. Returns `false` if the program isn't timed media.
    pub fn pause(&mut self) -> bool {
        self.set_playback(Playback::Paused)
    }

    /// Asks the stage to jump to `position`. Returns `false` if the program isn't timed media.
    pub fn seek(&mut self, position: Duration) -> bool {
        if !self.program_is_timed() {
            return false;
        }
        let position_ms = u64::try_from(position.as_millis()).unwrap_or(u64::MAX);
        self.emit(StageEvent::Seek { position_ms });
        true
    }

    fn set_playback(&mut self, playback: Playback) -> bool {
        if !self.program_is_timed() {
            return false;
        }
        if let Some(scene) = self.state.program.as_mut()
            && scene.playback != playback
        {
            scene.playback = playback;
            self.emit(StageEvent::Playback { playback });
        }
        true
    }

    fn program_is_timed(&self) -> bool {
        self.program()
            .is_some_and(|s| s.playback != Playback::Static)
    }

    fn set_program(&mut self, scene: Option<Scene>) {
        self.state.program = scene;
        self.emit(StageEvent::Program {
            scene: self.state.program.clone(),
        });
    }

    fn emit(&mut self, event: StageEvent) {
        self.subscribers.retain(|tx| tx.send(event.clone()).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use chorquest_resource::{Kind, Location, ResourceId, SourceId};

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
        stage.show(resource(Kind::Document, Location::File("/a.pdf".into())));
        assert!(!stage.next_frame());
        stage.set_frame_count(3);
        assert!(stage.next_frame());
        assert!(stage.next_frame());
        stage.set_frame_count(2);
        assert_eq!(stage.program().unwrap().frame, 1);
    }

    #[test]
    fn take_moves_preview_to_program() {
        let mut stage = Showman::new();
        assert!(stage.take().is_none());
        stage.preview(video());
        assert!(stage.program().is_none());
        assert!(stage.take().is_some());
        assert!(stage.state().preview.is_none());
        assert_eq!(stage.program().unwrap().resource.kind, Kind::Video);
    }

    #[test]
    fn playback_only_for_timed_media() {
        let mut stage = Showman::new();
        stage.show(hymn());
        assert!(!stage.play());
        assert!(!stage.seek(Duration::from_secs(1)));
        stage.show(video());
        assert_eq!(stage.program().unwrap().playback, Playback::Paused);
        assert!(stage.play());
        assert_eq!(stage.program().unwrap().playback, Playback::Playing);
    }

    #[test]
    fn subscribers_see_events_and_can_leave() {
        let mut stage = Showman::new();
        let rx = stage.subscribe();
        let gone = stage.subscribe();
        drop(gone);

        stage.show(hymn());
        stage.next_frame();
        stage.toggle_blackout();
        stage.clear();
        stage.clear(); // no-op, no event

        let events: Vec<_> = rx.try_iter().collect();
        assert!(matches!(events[0], StageEvent::Program { scene: Some(_) }));
        assert_eq!(events[1], StageEvent::Frame { frame: 1 });
        assert_eq!(events[2], StageEvent::Blackout { on: true });
        assert_eq!(events[3], StageEvent::Program { scene: None });
        assert_eq!(events.len(), 4);
        assert_eq!(stage.subscribers.len(), 1);
    }

    #[test]
    fn events_serialize_with_a_tag() {
        let json = serde_json::to_string(&StageEvent::Frame { frame: 2 }).unwrap();
        assert_eq!(json, r#"{"event":"frame","frame":2}"#);
    }
}
