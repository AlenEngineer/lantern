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
    /// Searches for something to show. The results come in the snapshot.
    Search {
        /// What was typed, bang and all: `juan 3 16`, `!yt himno 99`. Empty
        /// ends the search.
        text: String,
        /// Whether the search was asked for outright, not just typed: a
        /// source that is asked only on request (YouTube, images) waits
        /// for this.
        #[serde(default)]
        enter: bool,
    },
    /// Puts a search result on stage.
    Show {
        /// The result's id, as in [`Hit::id`].
        id: String,
    },
    /// Lines a search result up to show next, without showing it: the
    /// dashboard's preview, taken with its Take.
    LineUp {
        /// The result's id, as in [`Hit::id`].
        id: String,
    },
    /// Lists a library folder's files, for its tile; none stops listing.
    Browse {
        /// Which, as in [`Source::folder`].
        folder: Option<usize>,
        /// Words its files' names must have; empty lists them all.
        #[serde(default)]
        filter: String,
    },
    /// Shows again something shown this session.
    Again {
        /// Which, as in [`Shown::n`].
        n: usize,
    },
    /// Adds a search result to the plan, after the step it is at.
    Add {
        /// The result's id, as in [`Hit::id`].
        id: String,
    },
    /// Shows something the slide on stage plays or links to: a video kept
    /// in the deck, a YouTube video, a picture.
    Attachment {
        /// Which, counted from 0 in [`Live::attached`].
        n: usize,
    },
    /// Jumps in what is playing.
    Seek {
        /// Where to, in seconds from its start.
        seconds: f64,
    },
    /// Sets the computer's volume, and hears it again if it was muted.
    Volume {
        /// How loud, from 0 to 1 (full), as the desktop's own control
        /// shows it.
        level: f32,
    },
    /// Mutes the computer's sound, or hears it again.
    Mute {
        /// Whether it is muted after.
        on: bool,
    },
}

/// Everything the page shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Snapshot {
    /// The app's colours, so the page looks like it.
    pub colors: Colors,
    /// The mode the computer is in. The phone is a remote in Run and
    /// Live; in Plan it only says the computer is planning.
    pub mode: Mode,
    /// When the plan starts, if it has a start time.
    pub starts: Option<Starts>,
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
    /// How the service stands: the section under way against its time,
    /// then how far behind or ahead the whole is.
    pub notes: Vec<Note>,
    /// The bangs a search can start with, each for one kind of thing.
    pub bangs: Vec<Bang>,
    /// The search a phone asked for, while there is one.
    pub search: Option<Found>,
    /// The computer's sound, where the phone can change it.
    pub sound: Option<Sound>,
    /// In Live, what to show from: a tile each.
    pub sources: Vec<Source>,
    /// In Live, the first Bible's books, for its picker.
    pub bible: Option<BibleBooks>,
    /// In Live, what was shown since Live began, oldest first.
    pub shown: Vec<Shown>,
    /// The folder the phone asked to list ([`Command::Browse`]).
    pub listed: Option<Listed>,
}

/// Something to show from, as a tile on the phone.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Source {
    /// What it is, for its picker.
    pub kind: SourceKind,
    /// Its name: "Bible", a folder's label.
    pub name: String,
    /// The bang that searches it, without its `!`.
    pub bang: String,
    /// A few words under the name: which Bible.
    pub hint: String,
    /// A folder's place among the library's, for [`Command::Browse`].
    pub folder: Option<usize>,
}

/// The sorts of source the phone has a picker for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Book, chapter and verse.
    #[default]
    Bible,
    /// A library folder: its files.
    Folder,
    /// YouTube videos.
    Youtube,
    /// Typed text.
    Text,
    /// The clock and countdowns.
    Time,
    /// Pictures from the web.
    Images,
}

/// A Bible's books, without their text.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct BibleBooks {
    /// The translation's short name: "RVG".
    pub name: String,
    /// Every book in order, Genesis first; the first 39 are the Old
    /// Testament.
    pub books: Vec<BibleBook>,
}

/// A book of the Bible.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct BibleBook {
    /// Its name, as a search reads it: "Juan".
    pub name: String,
    /// For its button: "Jn".
    pub short: String,
    /// How many verses each chapter has; empty when the translation
    /// lacks the book.
    pub verses: Vec<u16>,
}

/// Something shown this session.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Shown {
    /// What [`Command::Again`] takes.
    pub n: usize,
    /// When, as a clock says it: "19:02".
    pub at: String,
    /// Its title.
    pub title: String,
    /// Whether it is on stage now.
    pub live: bool,
}

/// A folder's files, in natural order.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Listed {
    /// Which folder, as in [`Source::folder`].
    pub folder: usize,
    /// The words asked for, as in [`Command::Browse`].
    pub filter: String,
    /// How many files the folder has.
    pub files: usize,
    /// How many of them have the words.
    pub matching: usize,
    /// The first of those, in name order: a big folder (a hymnal) is
    /// narrowed by typing, not sent whole.
    pub hits: Vec<Hit>,
}

/// The computer's mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The plan is being made.
    #[default]
    Plan,
    /// The plan is being run.
    Run,
    /// Things are shown as they're found.
    Live,
}

/// When the plan starts.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Starts {
    /// The time of day, as the plan says it: `10:30`.
    pub at: String,
    /// How long until then, in seconds; none once it has passed. The page
    /// counts down from it itself.
    pub in_seconds: Option<u64>,
}

/// How a section, or the service, is doing against its time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pace {
    /// On track, or nothing to hold it against.
    #[default]
    OnTrack,
    /// Only just fits.
    Tight,
    /// Won't fit.
    Over,
}

/// A few words on how the service stands: "Escuela Sabática · 4:10
/// left", "4 min behind".
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Note {
    /// The words.
    pub text: String,
    /// The pace they tell of, for their tint.
    pub pace: Pace,
}

/// The computer's sound output: what [`Command::Volume`] and
/// [`Command::Mute`] change.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Sound {
    /// The output's name ("HD Audio Speaker"); empty when it has none.
    pub output: String,
    /// How loud, from 0 to 1 (full). It can be over 1 where the desktop
    /// let it go past full.
    pub volume: f32,
    /// Whether it is muted.
    pub muted: bool,
}

/// A bang: `!b` asks the Bibles alone.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Bang {
    /// Without its `!`.
    pub bang: String,
    /// What it searches, for its button.
    pub name: String,
}

/// A search and what it found so far.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Found {
    /// What was searched for, as the phone sent it.
    pub text: String,
    /// Whether sources are still answering.
    pub busy: bool,
    /// Whether the search waits to be asked for outright
    /// ([`Command::Search`] with `enter`).
    pub waiting: bool,
    /// The results, by where they are from.
    pub groups: Vec<Group>,
}

/// The results from one source.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Group {
    /// The source's name.
    pub name: String,
    /// Why it couldn't answer, if it couldn't.
    pub error: Option<String>,
    /// What it found.
    pub hits: Vec<Hit>,
}

/// One search result, drawn as the computer draws it: a 16:9 picture
/// with its size in the lower right corner and its state in the upper
/// left, then the title and one line.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Hit {
    /// What [`Command::Show`] and [`Command::Add`] take.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Where it's from and what it is: "Himnario · video".
    pub facts: String,
    /// The words it shows, for a passage or typed text; empty otherwise.
    pub text: String,
    /// What it is, for the picture when there is none.
    pub kind: HitKind,
    /// The corner: "3:07", "12 slides", "8 pages", "1920×1080", "3 verses".
    pub size: Option<String>,
    /// The upper-left badge, only when it has one.
    pub state: Option<Badge>,
    /// The one warning that matters; shown instead of `facts`.
    pub warning: Option<Badge>,
    /// How far a download is, 0 to 1, while it runs.
    pub progress: Option<f32>,
    /// Width over height of its picture, once known, for Shapes.
    pub shape: Option<f32>,
    /// Whether `GET /thumb.jpg?t=TOKEN&id=ID` has its picture now.
    pub picture: bool,
}

/// What a result is, for the picture drawn when it has none of its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HitKind {
    /// A video, a YouTube video among them.
    Video,
    /// A picture.
    Picture,
    /// A deck of slides.
    Deck,
    /// A PDF.
    Pdf,
    /// A sound.
    Sound,
    /// A Bible passage.
    Passage,
    /// Typed text, a song's words.
    Text,
    /// The clock or a countdown.
    Time,
    /// A web page.
    Web,
    /// Anything else.
    #[default]
    Other,
}

/// A few words on a result, in a tint.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Badge {
    /// The words: "● live", "↓ 40%", "not downloaded yet".
    pub text: String,
    /// Their tint.
    pub tone: Tone,
}

/// A badge's tint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// [`Colors::text`].
    #[default]
    Plain,
    /// [`Colors::live`]: on stage, or a stream that needs the internet
    /// throughout.
    Live,
    /// [`Colors::live`] too, the amber of the computer's warnings: it
    /// shows, but badly or not yet.
    Waiting,
    /// [`Colors::danger`]: it won't show.
    Problem,
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
    /// A few words at the row's end: its length, "live", "black"; a
    /// section's budget.
    pub detail: String,
    /// A section's pace while it runs or once done.
    pub pace: Pace,
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
    /// What the slide on stage plays or links to, a name each ("Video",
    /// "YouTube video 2"): [`Command::Attachment`] shows one.
    pub attached: Vec<String>,
    /// The slide of the deck this was shown from, counted from 1, while
    /// there is one to go back to: [`Command::Prev`] goes back to it and
    /// [`Command::Next`] on to the slide after.
    pub back: Option<usize>,
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
    /// How long it takes, when that's known.
    pub detail: String,
}
