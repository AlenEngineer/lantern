use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use lantern_resource::{Kind, Location, Resource};
use serde::{Deserialize, Serialize};

/// Names one scene for its whole life.
///
/// The [`Showman`](crate::Showman) is the only code that makes ids, and it
/// keeps two rules:
/// - an id is never reused, even after the stage is cleared;
/// - an id never changes slot. [`take`](crate::Showman::take) gives the new
///   program scene a fresh id, so a report meant for the preview can never
///   land on program.
///
/// So a report tagged with an id reaches the scene it was made for, or
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SceneId(u64);

impl SceneId {
    pub(crate) fn new(raw: u64) -> Self {
        Self(raw)
    }
}

impl fmt::Display for SceneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "scene #{}", self.0)
    }
}

/// A resource as it is being presented, in program or in preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// Which scene this is. Renderers tag their reports with it.
    pub id: SceneId,
    /// What is presented.
    pub resource: Arc<Resource>,
    /// How it is presented right now.
    pub media: Media,
}

/// How a scene is presented. Exactly one shape at a time, so "playing, on
/// frame 3 of 5" can't be represented.
///
/// The engine guesses the shape when the scene is made. The renderer is the
/// authority, and its [`MediaReport`]s replace the guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Media {
    /// Stanzas, pages or slides. A still image or a web page is one frame.
    Frames {
        /// Current frame, from 0. Below `count` whenever `count` is known.
        frame: usize,
        /// `None` until the stage reports it (documents, decks). Until then
        /// the scene behaves as a single frame.
        count: Option<NonZeroUsize>,
    },
    /// Audio or video on a timeline.
    Timed(Timed),
    /// The stage reported that it can't present the scene. There are no
    /// frames to step through and nothing to play. A later `Frames` or
    /// `Timeline` report (a retry that worked) replaces it.
    Failed {
        /// What the stage said, for the operator.
        reason: String,
    },
}

/// The timeline of a timed scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timed {
    /// Starts `Paused`, because pressing play is always a deliberate act.
    pub playback: Playback,
    /// The last position the stage reported, or the target of the last seek,
    /// whichever came later. While playing it moves only when the stage
    /// reports, so a dashboard may extrapolate between reports.
    #[serde(rename = "position_ms", with = "millis")]
    pub position: Duration,
    /// `None` until the stage reports it, and for streams without an end.
    #[serde(rename = "duration_ms", with = "millis_opt")]
    pub duration: Option<Duration>,
}

/// Where a timed scene is in its life.
///
/// The operator moves between `Paused` and `Playing`. Only the stage's
/// [`MediaReport::Ended`] reaches `Ended`. From `Ended`, `play` replays from
/// the start and a seek returns to `Paused`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Playback {
    /// Paused, or not started. A scene still loading is paused.
    Paused,
    /// Playing.
    Playing,
    /// Reached the end.
    Ended,
}

/// What a renderer learned about one scene's media, sent with the scene's id.
///
/// Every report states an absolute fact, so sending one twice changes
/// nothing and a lost report is fixed by the next one. A report for a scene
/// that has left the stage is ignored.
///
/// Serializable so a webview stage can send reports across a process
/// boundary. Externally tagged, because two variants wrap a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaReport {
    /// The stage presents the scene as this many frames: the pages of a PDF,
    /// the slides of a deck, or the slides inline text needed once split
    /// with [`fit_frames`]. The current frame is clamped to the count.
    /// Ignored for timed scenes (a stage that can't play one reports
    /// `Failed`).
    Frames(NonZeroUsize),
    /// The stage plays the scene on a timeline of this length. A frames
    /// scene becomes timed and starts paused, so the stage must hold it
    /// paused until a `Playback` event says otherwise. Ignored for inline
    /// text.
    Timeline {
        /// `None` for a stream without an end.
        #[serde(rename = "duration_ms", with = "millis_opt")]
        duration: Option<Duration>,
    },
    /// Where playback is now. Send it about four times a second while
    /// playing, and once after a seek or a pause settles. Ignored once the
    /// scene has ended, so a report already in flight at the end of the
    /// video can't pull it back.
    Position(#[serde(with = "millis")] Duration),
    /// Playback reached the end.
    Ended,
    /// The stage can't present the scene. Its stage window shows black, and
    /// the reason goes to the operator. Play, pause and seek are refused
    /// from then on.
    Failed {
        /// Shown to the operator.
        reason: String,
    },
}

impl Scene {
    pub(crate) fn new(id: SceneId, resource: Arc<Resource>) -> Self {
        let media = Media::guess(&resource);
        Self {
            id,
            resource,
            media,
        }
    }

    /// The current frame. 0 for timed and failed scenes.
    pub fn frame(&self) -> usize {
        match self.media {
            Media::Frames { frame, .. } => frame,
            Media::Timed(_) | Media::Failed { .. } => 0,
        }
    }

    /// Whether there is a frame after the current one. An unknown count
    /// behaves as one frame.
    pub fn has_next_frame(&self) -> bool {
        match self.media {
            Media::Frames { frame, count } => frame + 1 < count.map_or(1, NonZeroUsize::get),
            Media::Timed(_) | Media::Failed { .. } => false,
        }
    }

    /// The text of the current frame, for inline text, split only at blank
    /// lines. A stage that measures uses [`Scene::text_fitting`].
    pub fn text(&self) -> Option<String> {
        self.text_fitting(|_| true)
    }

    /// The text of the current frame, for inline text, split with
    /// [`fit_frames`] by what `fits` on the stage.
    pub fn text_fitting(&self, fits: impl Fn(&str) -> bool) -> Option<String> {
        let body = self.resource.location.as_text()?;
        let frames = fit_frames(body, fits);
        let last = frames.len().checked_sub(1)?;
        frames.into_iter().nth(self.frame().min(last))
    }

    /// The timeline, for timed scenes.
    pub fn timed(&self) -> Option<&Timed> {
        match &self.media {
            Media::Timed(timed) => Some(timed),
            Media::Frames { .. } | Media::Failed { .. } => None,
        }
    }
}

impl Media {
    /// The engine's guess before any renderer has spoken. The one place that
    /// decides frames versus timeline from a resource.
    fn guess(resource: &Resource) -> Media {
        match (&resource.location, &resource.kind) {
            (Location::Inline(body), _) => Media::Frames {
                frame: 0,
                count: Some(
                    NonZeroUsize::new(text_frames(body).len()).unwrap_or(NonZeroUsize::MIN),
                ),
            },
            (_, Kind::Video | Kind::Audio) => Media::Timed(Timed::start(None)),
            (_, Kind::Image | Kind::Web) => Media::Frames {
                frame: 0,
                count: Some(NonZeroUsize::MIN),
            },
            (_, Kind::Document | Kind::Presentation | Kind::Text | Kind::Other(_)) => {
                Media::Frames {
                    frame: 0,
                    count: None,
                }
            }
        }
    }

    /// What `report` turns this media into, or `None` if the report doesn't
    /// apply to it. The `Showman` broadcasts the result, so mirrors never
    /// re-run this policy.
    pub(crate) fn after(&self, resource: &Resource, report: &MediaReport) -> Option<Media> {
        use MediaReport as R;
        let inline = resource.location.as_text().is_some();
        match (self, report) {
            // Inline text is slides: the stage may say how many it needed,
            // but never that it plays.
            (_, R::Timeline { .. }) if inline => None,

            (Media::Frames { frame, .. }, R::Frames(count)) => Some(Media::Frames {
                frame: (*frame).min(count.get() - 1),
                count: Some(*count),
            }),
            (Media::Failed { .. }, R::Frames(count)) => Some(Media::Frames {
                frame: 0,
                count: Some(*count),
            }),
            (Media::Timed(_), R::Frames(_)) => None,

            (Media::Timed(timed), R::Timeline { duration }) => Some(Media::Timed(Timed {
                duration: *duration,
                ..*timed
            })),
            (Media::Frames { .. } | Media::Failed { .. }, R::Timeline { duration }) => {
                Some(Media::Timed(Timed::start(*duration)))
            }

            (Media::Timed(timed), R::Position(position)) if timed.playback != Playback::Ended => {
                Some(Media::Timed(Timed {
                    position: *position,
                    ..*timed
                }))
            }
            (Media::Timed(timed), R::Ended) => Some(Media::Timed(Timed {
                playback: Playback::Ended,
                position: timed.duration.unwrap_or(timed.position),
                ..*timed
            })),

            (_, R::Failed { reason }) => Some(Media::Failed {
                reason: reason.clone(),
            }),

            (_, R::Position(_) | R::Ended) => None,
        }
    }
}

impl Timed {
    fn start(duration: Option<Duration>) -> Self {
        Self {
            playback: Playback::Paused,
            position: Duration::ZERO,
            duration,
        }
    }

    /// Moves the playhead. A timeline moved off its end is no longer ended.
    pub(crate) fn seek(&mut self, position: Duration) {
        self.position = position;
        if self.playback == Playback::Ended {
            self.playback = Playback::Paused;
        }
    }
}

/// Splits text into slides that each `fits` the stage: first at blank
/// lines (stanzas, verses), then a stanza that doesn't fit at its lines,
/// a line that doesn't at its sentences, and a sentence that doesn't at
/// its words. Each slide takes as many pieces as fit, in order. A single
/// word that doesn't fit is a slide of its own.
///
/// `fits` must say yes to every piece of something it says yes to.
pub fn fit_frames(body: &str, fits: impl Fn(&str) -> bool) -> Vec<String> {
    let mut slides = Vec::new();
    for stanza in text_frames(body) {
        if fits(&stanza) {
            slides.push(stanza);
        } else {
            slides.extend(pack(&stanza, Cut::Lines, &fits));
        }
    }
    slides
}

/// How a text that doesn't fit is cut, coarsest first.
#[derive(Clone, Copy)]
enum Cut {
    Lines,
    Sentences,
    Words,
}

impl Cut {
    fn finer(self) -> Option<Cut> {
        match self {
            Cut::Lines => Some(Cut::Sentences),
            Cut::Sentences => Some(Cut::Words),
            Cut::Words => None,
        }
    }

    /// The pieces of `text`, and what joins two of them on one slide.
    fn pieces(self, text: &str) -> (Vec<&str>, &'static str) {
        match self {
            Cut::Lines => (
                text.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .collect(),
                "\n",
            ),
            Cut::Sentences => (sentences(text), " "),
            Cut::Words => (text.split_whitespace().collect(), " "),
        }
    }
}

/// `text`, which doesn't fit, as slides that do, cutting at `cut` or
/// finer. Measuring is the cost, so each slide's run of pieces is found
/// by doubling it until it doesn't fit and then halving back: about
/// 2·log(n) measures a slide rather than one a piece.
fn pack(text: &str, cut: Cut, fits: &impl Fn(&str) -> bool) -> Vec<String> {
    let (pieces, join) = cut.pieces(text);
    if pieces.len() < 2 {
        return match cut.finer() {
            Some(finer) => pack(text, finer, fits),
            None => vec![text.to_string()],
        };
    }
    let run = |from: usize, to: usize| pieces[from..to].join(join);
    let mut slides = Vec::new();
    let mut cut_finer = false;
    let mut from = 0;
    while from < pieces.len() {
        if !fits(pieces[from]) {
            cut_finer = true;
            match cut.finer() {
                Some(finer) => slides.extend(pack(pieces[from], finer, fits)),
                None => slides.push(pieces[from].to_string()),
            }
            from += 1;
            continue;
        }
        // `good` pieces fit; `bad` don't, or run past the end.
        let (mut good, mut bad) = (1, 2);
        while from + bad <= pieces.len() && fits(&run(from, from + bad)) {
            good = bad;
            bad *= 2;
        }
        bad = bad.min(pieces.len() - from + 1);
        while bad - good > 1 {
            let middle = (good + bad) / 2;
            if fits(&run(from, from + middle)) {
                good = middle;
            } else {
                bad = middle;
            }
        }
        slides.push(run(from, from + good));
        from += good;
    }
    // Filling each slide in turn leaves the last one short, so its text
    // would be set much larger. The same number of slides cut at even
    // lengths reads evenly, when that shortens the longest slide and they
    // all still fit.
    if !cut_finer && slides.len() > 1 {
        let even: Vec<String> = even_cuts(&pieces, slides.len())
            .windows(2)
            .map(|cut| run(cut[0], cut[1]))
            .collect();
        let longest = |slides: &[String]| slides.iter().map(|s| s.chars().count()).max();
        if longest(&even) < longest(&slides) && even.iter().all(|slide| fits(slide)) {
            return even;
        }
    }
    slides
}

/// Where to cut `pieces` into `count` runs of about the same length in
/// characters: the run boundaries, from 0 to `pieces.len()`.
fn even_cuts(pieces: &[&str], count: usize) -> Vec<usize> {
    let lengths: Vec<usize> = pieces.iter().map(|p| p.chars().count() + 1).collect();
    let total: usize = lengths.iter().sum();
    let mut cuts = vec![0];
    let mut so_far: usize = 0;
    for (at, length) in lengths.iter().enumerate() {
        let wanted = total * cuts.len() / count;
        let left = pieces.len() - at;
        let runs_left = count - cuts.len();
        // Cut before this piece if that lands nearer the wanted length,
        // or if every remaining run needs a piece of its own.
        if cuts.len() < count
            && at > *cuts.last().unwrap_or(&0)
            && (so_far.abs_diff(wanted) <= (so_far + length).abs_diff(wanted) || left == runs_left)
        {
            cuts.push(at);
        }
        so_far += length;
    }
    cuts.push(pieces.len());
    cuts
}

/// `text` cut after each `.`, `!`, `?`, `;` or `:` that a space follows,
/// the punctuation kept with its sentence.
fn sentences(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let ends = matches!(c, '.' | '!' | '?' | ';' | ':')
            && chars.peek().is_some_and(|(_, next)| next.is_whitespace());
        if ends {
            let end = at + c.len_utf8();
            found.push(text[start..end].trim());
            start = end;
        }
    }
    found.push(text[start..].trim());
    found.retain(|s| !s.is_empty());
    found
}

/// Splits text into slides at blank lines. Lines within a slide are kept;
/// surrounding whitespace is trimmed.
pub fn text_frames(body: &str) -> Vec<String> {
    let mut frames = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in body.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                frames.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line.trim_end());
        }
    }
    if !current.is_empty() {
        frames.push(current.join("\n"));
    }
    frames
}

/// Durations cross the wire as whole milliseconds.
pub(crate) mod millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        to_millis(*value).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        u64::deserialize(deserializer).map(Duration::from_millis)
    }

    pub(super) fn to_millis(value: Duration) -> u64 {
        u64::try_from(value.as_millis()).unwrap_or(u64::MAX)
    }
}

/// [`millis`] for `Option<Duration>`.
pub(crate) mod millis_opt {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<Duration>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.map(super::millis::to_millis).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Duration>, D::Error> {
        Option::<u64>::deserialize(deserializer).map(|ms| ms.map(Duration::from_millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_blank_lines() {
        let body = "Line 1\nLine 2\r\n\r\n   \nChorus\n\n";
        assert_eq!(text_frames(body), ["Line 1\nLine 2", "Chorus"]);
        assert!(text_frames("\n \n").is_empty());
    }

    /// Fits when no line is longer than `width` and there are at most
    /// `lines` lines: a stand-in for a renderer's measure.
    fn fits(width: usize, lines: usize) -> impl Fn(&str) -> bool {
        move |text: &str| {
            text.lines().count() <= lines && text.lines().all(|l| l.chars().count() <= width)
        }
    }

    #[test]
    fn text_that_fits_keeps_its_stanzas() {
        let body = "Line 1\nLine 2\n\nChorus";
        assert_eq!(fit_frames(body, fits(80, 10)), text_frames(body));
    }

    #[test]
    fn a_long_stanza_splits_at_lines_then_sentences_then_words() {
        let stanza = "one\ntwo\nthree\nfour\nfive";
        let counted = std::cell::Cell::new(0);
        let counting = |text: &str| {
            counted.set(counted.get() + 1);
            fits(80, 2)(text)
        };
        fit_frames(stanza, counting);
        // Doubling and halving per slide, then one check per evened slide.
        assert!(counted.get() <= 12, "measured {} times", counted.get());
        assert_eq!(
            fit_frames(stanza, fits(80, 2)),
            ["one\ntwo", "three", "four\nfive"]
        );

        let verse = "Porque de tal manera amó Dios al mundo. Que ha dado á su Hijo. Para que todo aquel crea.";
        assert_eq!(
            fit_frames(verse, fits(42, 1)),
            [
                "Porque de tal manera amó Dios al mundo.",
                "Que ha dado á su Hijo.",
                "Para que todo aquel crea."
            ]
        );
        assert_eq!(
            fit_frames("uno dos tres cuatro", fits(8, 1)),
            ["uno dos", "tres", "cuatro"]
        );
        assert_eq!(
            fit_frames("larguísima", fits(3, 1)),
            ["larguísima"],
            "a word that doesn't fit"
        );
    }

    #[test]
    fn slides_come_out_about_even() {
        let short = |text: &str| text.chars().count() <= 20;
        assert_eq!(
            fit_frames("aaaa bbbb cccc dddd eeee ffff", short),
            ["aaaa bbbb cccc", "dddd eeee ffff"],
            "not 4 + 2"
        );
        assert_eq!(even_cuts(&["a", "b", "c"], 3), [0, 1, 2, 3]);
        assert_eq!(even_cuts(&["long piece", "a", "b"], 2), [0, 1, 3]);
    }

    #[test]
    fn sentences_keep_their_punctuation_and_skip_decimals() {
        assert_eq!(
            sentences("¡Gloria! ¿Quién? Él: sí; 3.14 no."),
            ["¡Gloria!", "¿Quién?", "Él:", "sí;", "3.14 no."]
        );
    }

    #[test]
    fn a_stage_can_say_how_many_slides_a_text_needed() {
        use lantern_resource::{ResourceId, SourceId};
        let source = SourceId::new("test").unwrap();
        let resource = Resource::new(
            ResourceId::new(&source, "t"),
            "T",
            Kind::Text,
            Location::Inline("a b c d".into()),
        );
        let media = Media::guess(&resource);
        let three = NonZeroUsize::new(3).unwrap();
        assert_eq!(
            media.after(&resource, &MediaReport::Frames(three)),
            Some(Media::Frames {
                frame: 0,
                count: Some(three)
            })
        );
        assert_eq!(
            media.after(&resource, &MediaReport::Timeline { duration: None }),
            None
        );
    }
}
