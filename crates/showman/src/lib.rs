//! The stage: what the audience sees.
//!
//! chorquest runs in two windows. The *dashboard* is for the operator and
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
//! Inside a scene there are *frames*: stanzas of a hymn, pages of a PDF,
//! slides of a deck. Timed media (audio, video) also has a playback state.
//!
//! Windows don't poll: they [`subscribe`](Showman::subscribe) and react to
//! [`StageEvent`]s. A window that opens late starts from [`Showman::state`].

mod scene;
mod showman;

pub use scene::{Playback, Scene, text_frames};
pub use showman::{Showman, StageEvent, StageState};
