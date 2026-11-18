//! The order of things.
//!
//! A [`Plan`] is what you prepare before the event: an ordered list of
//! [`Step`]s. A step either shows something concrete ([`Action::Show`]),
//! reserves a slot where you'll search live ([`Action::Live`]), blanks the
//! screen ([`Action::Blank`]) or is just a reminder for you ([`Action::Note`]).
//!
//! A [`Session`] is the plan while it runs: a cursor over the steps plus a
//! log of everything that was actually shown, planned or not. Planned and
//! live work can be mixed freely: showing something off-plan never loses
//! your place.
//!
//! [`Action::Section`] markers split a plan into parts with a time budget,
//! and [`timing`] reports how each part is keeping to it. Sections only
//! name parts: what comes again, or in an order drawn afresh, is a
//! [`Span`] laid over some of the steps.
//!
//! This crate knows nothing about screens; the engine connects it to the stage.

mod note;
mod plan;
mod session;
pub mod timing;

pub use note::{Note, NoteId, NoteOn};
pub use plan::{
    Action, BracketError, Carried, Carry, CarryError, DEEPEST, Drawn, Ending, Flow, FlowOf, Looped,
    Over, Place, Plan, PlanError, Repeat, Span, SpanError, SpanId, Step, StepId, StepSound,
};
pub use session::{LogEntry, Session};
