//! What passes between the app and the page: commands one way, snapshots
//! the other. Both are JSON, and the page's script reads these names.

use serde::{Deserialize, Serialize};

/// What a phone asks for. `{"do": "next"}`, `{"do": "go", "step": 7}`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Command {
    /// The phone has just arrived and wants a snapshot. The server sends
    /// this itself; a phone needn't.
    Hello,
    /// The next slide, or the plan's next step.
    Next,
    /// The slide or step before.
    Prev,
    /// Plays or pauses what is on stage.
    PlayPause,
    /// Turns the blackout on or off.
    Blackout,
    /// Empties the stage.
    Clear,
    /// Goes back to the plan's step after something off it was shown.
    Resume,
    /// Goes to a step of the plan.
    Go {
        /// The step's id, as in [`Step::id`].
        step: u64,
    },
    /// Jumps in what is playing.
    Seek {
        /// Where to, in seconds from its start.
        seconds: f64,
    },
}

/// Everything the page shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Snapshot {
    /// The app's colours, so the page looks like it.
    pub colors: Colors,
    /// The open plan.
    pub plan: Option<Plan>,
    /// What is on stage.
    pub live: Option<Live>,
    /// What comes after it.
    pub next: Option<Upcoming>,
    /// Whether the stage is blacked out.
    pub blackout: bool,
    /// Whether what is on stage isn't the plan's step: there is a plan to
    /// go back to.
    pub off_plan: bool,
    /// How the service stands, a few words each: "Escuela Sabática ·
    /// 4:10 / 30m", "4 min behind".
    pub notes: Vec<String>,
}

/// The app's colours, as `#rrggbb`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Colors {
    /// Behind everything.
    pub ground: String,
    /// A card on the ground.
    pub card: String,
    /// Something raised on a card: a button, the row picked.
    pub raised: String,
    /// Text.
    pub text: String,
    /// Text that matters less.
    pub dim: String,
    /// The accent, and what is written on it.
    pub accent: String,
    /// See [`accent`](Colors::accent).
    pub on_accent: String,
    /// What is on stage now.
    pub live: String,
    /// What went wrong, and the blackout.
    pub danger: String,
}

/// The open plan.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Plan {
    /// Its name.
    pub name: String,
    /// Its steps, sections among them, in order.
    pub steps: Vec<Step>,
}

/// One row of the plan.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Step {
    /// What [`Command::Go`] takes.
    pub id: u64,
    /// Its name.
    pub label: String,
    /// What it is.
    pub kind: StepKind,
    /// Where the service is against it.
    pub place: Place,
    /// A few words at the row's end: its length, "live", "black".
    pub detail: String,
    /// Why it can't be shown, if it can't.
    pub problem: Option<String>,
}

/// What a step is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// Shows something.
    #[default]
    Show,
    /// A heading for the steps under it.
    Section,
    /// A slot for showing things on the fly.
    Live,
    /// Empties the stage.
    Blank,
}

/// Where the service is against a step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Place {
    /// Behind it.
    Done,
    /// At it.
    Current,
    /// Still to come.
    #[default]
    Ahead,
}

/// What is on stage.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Live {
    /// Its title.
    pub title: String,
    /// A line under it: where it is from, how far in.
    pub facts: String,
    /// Its timeline, for a video.
    pub timed: Option<Timed>,
    /// Its slides or pages, when it has several.
    pub slides: Option<Slides>,
    /// Why the stage can't show it, if it can't.
    pub failed: Option<String>,
}

/// A video's timeline.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Timed {
    /// Whether it is playing. The page counts on from `position` itself.
    pub playing: bool,
    /// Whether it reached its end.
    pub ended: bool,
    /// Where it is, in whole seconds.
    pub position: u64,
    /// Its length in seconds; none for a live video.
    pub length: Option<u64>,
}

/// Where the stage is in something with several slides.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Slides {
    /// The slide shown, counted from 1.
    pub at: usize,
    /// How many there are, once known.
    pub of: Option<usize>,
}

/// What comes next.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Upcoming {
    /// Its title.
    pub title: String,
}
