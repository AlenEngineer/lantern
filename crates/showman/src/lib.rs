//! The stage: what the audience sees.
//!
//! lantern runs in two windows. The *dashboard* is for the operator and
//! can show anything. The *stage* window is what goes to the projector, and
//! it must only ever show what the operator decided to put there. This crate
//! models that second window, without drawing anything:
//!
//! - **Program** is what is on stage right now.
//! - **Preview** is what the operator has lined up. [`Showman::take`] moves
//!   it to program (like a video switcher), so a live search result can be
//!   checked before the audience sees it.
//! - **Blackout** hides the stage without forgetting what was on it.
//!
//! Each [`Scene`] has a [`SceneId`] and a [`Media`]: frames (stanzas of a
//! hymn, pages of a PDF, slides of a deck), a timeline (audio, video), or a
//! failure. Renderers send [`MediaReport`]s about one scene by its id.
//!
//! A [`Look`] says how the stage looks: its background, and how text is
//! set (font, colour, the size range text fits within).
//!
//! Windows don't poll. [`Showman::subscribe`] returns a [`StageFeed`], a
//! snapshot and the events after it. A window applies each event to its
//! copy with [`StageState::apply`], the same function the `Showman` uses.

mod look;
mod scene;
mod showman;

pub use look::{Look, Rgb};
pub use scene::{Media, MediaReport, Playback, Scene, SceneId, Timed, fit_frames, text_frames};
pub use showman::{Showman, StageEvent, StageFeed, StageState};
