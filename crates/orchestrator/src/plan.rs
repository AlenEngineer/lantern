use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lantern_resource::{Query, Resource};
use lantern_showman::{Look, LookChange};
use serde::{Deserialize, Serialize};

use crate::timing::TimeOfDay;

/// Errors loading or saving a plan.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    /// Reading or writing the file failed.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// The file is not a valid plan.
    #[error("invalid plan: {0}")]
    Format(#[from] serde_json::Error),
}

/// Identifies a step within its plan. Stays the same when steps are reordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StepId(pub(crate) u64);

impl fmt::Display for StepId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// What a step does when the session reaches it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Put this resource on stage.
    Show {
        /// What to show.
        resource: Resource,
    },
    /// A slot for searching and showing things on the fly. The stage is left
    /// as it is when the slot starts.
    Live {
        /// Optional preset search, e.g. "videos only".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hint: Option<Query>,
        /// What was shown during this slot, filled in while the session runs.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        shown: Vec<Resource>,
    },
    /// Clear the stage (prayer, sermon without slides...).
    Blank,
    /// A reminder for the operator only. Leaves the stage as it is.
    Note,
    /// Starts a section: the steps after it, up to the next section or
    /// to its own end, are one part of the service, such as the music or
    /// the sermon. Stepping through the plan passes over it. A section is
    /// a name over some steps, with a time and what its steps do unless
    /// they say: how the plan moves is not its to say, but a [`Span`]'s.
    Section {
        /// How long the section should take.
        #[serde(default, with = "seconds", skip_serializing_if = "Option::is_none")]
        budget: Option<Duration>,
        /// How far in it sits: 0 is a part of the plan, 1 a part of the
        /// section before it that sits less far in, and so on to
        /// [`DEEPEST`]. A section holds what follows it, up to the next
        /// section no further in than itself, or to its `last`.
        #[serde(default, skip_serializing_if = "is_zero")]
        depth: u8,
        /// The last row it holds, when it ends before the next section
        /// no further in than itself: what follows is back in whatever
        /// holds it. Its own heading for a section that holds nothing.
        /// Without one it runs to that next section, or until what holds
        /// it ends. See [`Plan::reach_section`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last: Option<StepId>,
        /// The time of day it should be over by, such as 11:45.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        done_by: Option<TimeOfDay>,
    },
}

/// The furthest in a section sits: three levels of section in all.
pub const DEEPEST: u8 = 2;

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(depth: &u8) -> bool {
    *depth == 0
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_none_yet(next: &u64) -> bool {
    *next == 0
}

/// How many times a step, a span of steps or the plan comes before the
/// plan moves past it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    /// Once: the usual.
    #[default]
    Once,
    /// This many times in all.
    Times(u16),
    /// Until the operator goes somewhere else.
    Forever,
    /// Again while it is before this time of day. How it ends when the
    /// time comes is [`Flow::ends`].
    Until(crate::timing::TimeOfDay),
}

/// How something that comes again until a time of day ends when that
/// time comes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ending {
    /// Once round is finished first: it ends at the time or after it,
    /// with nothing cut short.
    #[default]
    Round,
    /// The step that is on is finished, and the rest of the round is
    /// left out.
    Step,
    /// There and then: what is on is cut off.
    Sharp,
}

impl Ending {
    fn is_round(&self) -> bool {
        *self == Ending::Round
    }
}

/// How a step, a span of steps or the plan moves: how often it comes,
/// and for a span or the plan, in what order its steps do. Unlike
/// [`Step::advance`], none of it carries down: it is about the one thing
/// it is set on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Flow {
    /// How many times it comes.
    #[serde(skip_serializing_if = "Repeat::is_once")]
    pub repeat: Repeat,
    /// Its steps come in an order drawn afresh each time round.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub shuffle: bool,
    /// How it ends, coming again until a time of day.
    #[serde(skip_serializing_if = "Ending::is_round")]
    pub ends: Ending,
}

impl Repeat {
    fn is_once(&self) -> bool {
        *self == Repeat::Once
    }

    /// How many times it comes, for adding up how long a plan takes: a
    /// repeat with no number to it counts once, and is [`open`].
    ///
    /// [`open`]: Repeat::open
    pub fn times(self) -> u32 {
        match self {
            Repeat::Times(times) => u32::from(times.max(1)),
            Repeat::Once | Repeat::Forever | Repeat::Until(_) => 1,
        }
    }

    /// Whether how often it comes isn't a number: until the operator
    /// goes elsewhere, or until a time.
    pub fn open(self) -> bool {
        matches!(self, Repeat::Forever | Repeat::Until(_))
    }
}

impl Flow {
    /// Once and in order: nothing to keep.
    pub fn is_usual(&self) -> bool {
        *self == Flow::default()
    }
}

/// Identifies a span within its plan. Stays the same as its steps move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpanId(pub(crate) u64);

impl fmt::Display for SpanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "~{}", self.0)
    }
}

/// What a span is laid over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Over {
    /// From one step to another, and all that is between them wherever
    /// they move to. Neither is a section's heading.
    Steps {
        /// The first of them, as the plan runs.
        first: StepId,
        /// The last of them.
        last: StepId,
    },
    /// Everything the section with this heading holds, whatever that
    /// comes to be: a step put into the section later is in the span.
    Section(StepId),
}

/// A loop or a drawn order laid over a run of the plan's steps. The plan
/// is one sequence of steps, and its sections only name parts of it: a
/// span is what has some of them come again, or in an order drawn
/// afresh. See [`Plan::loop_over`] for the limits spans keep to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// Stable id.
    pub id: SpanId,
    /// The steps it is over.
    pub over: Over,
    /// How often they come, and in what order.
    pub flow: Flow,
}

impl Span {
    /// Whether its steps come again: a loop.
    pub fn is_loop(&self) -> bool {
        self.flow.repeat != Repeat::Once
    }

    /// Whether its steps come in an order drawn afresh each time round.
    pub fn is_drawn(&self) -> bool {
        self.flow.shuffle
    }
}

/// What a [`Flow`] is set on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowOf {
    /// The whole plan.
    Plan,
    /// A step by itself. For a section's heading, the span attached to
    /// that section: everything it holds.
    Step(StepId),
    /// A span.
    Span(SpanId),
}

impl From<Option<StepId>> for FlowOf {
    /// A step, or the plan for `None`.
    fn from(of: Option<StepId>) -> Self {
        of.map_or(FlowOf::Plan, FlowOf::Step)
    }
}

impl From<StepId> for FlowOf {
    fn from(id: StepId) -> Self {
        FlowOf::Step(id)
    }
}

impl From<SpanId> for FlowOf {
    fn from(id: SpanId) -> Self {
        FlowOf::Span(id)
    }
}

/// What drawing a loop by hand did: see [`Plan::loop_by_hand`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Looped {
    /// A new loop, of its own.
    Made(SpanId),
    /// One that was there, now round more steps.
    Grown(SpanId),
}

/// What drawing an order by hand did: see [`Plan::draw_over`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawn {
    /// A new order, of its own.
    Made(SpanId),
    /// One that was there, now over more steps: it took in what it met.
    Grown(SpanId),
    /// A part of one that was there was rubbed out.
    Rubbed,
}

/// Why a span can't be where it was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SpanError {
    /// A step, a section or a span it names isn't in the plan.
    #[error("that is no longer in the plan")]
    Missing,
    /// A loop needs two steps, and a drawn order three.
    #[error("too few steps: a loop goes round two or more, a drawn order three or more")]
    TooFew,
    /// A drawn order stays in one section: no heading, nor where a
    /// section ends, comes among its steps.
    #[error("a drawn order can't cross a section's heading or end")]
    AcrossHeading,
    /// It would take in a part of the span it names: two spans are
    /// apart, or one is wholly in the other.
    #[error("it would cut another loop or drawn order ({0}) in two")]
    HalfOver(SpanId),
}

/// Why a section can't be laid where it was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BracketError {
    /// A step or a section it names isn't in the plan, or isn't a section
    /// where one is wanted.
    #[error("that is no longer in the plan")]
    Missing,
    /// The last row asked for comes before the section's heading.
    #[error("a section can't end before its heading")]
    EndsBefore,
    /// It would take in a part of this section: two sections are apart,
    /// or one is wholly in the other.
    #[error("it would cut a section ({0}) in two")]
    HalfOver(StepId),
    /// A section would be further in than [`DEEPEST`].
    #[error("sections go no more than three deep")]
    TooDeep,
    /// A drawn order would have a heading or a section's end among its
    /// steps, which would end it.
    #[error("it would cut a drawn order ({0})")]
    Cuts(SpanId),
}

/// Why sections can't be laid out as asked, inside the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Misfit {
    /// A section would end before its heading.
    Backwards,
    /// Of these two sections, the second would start in the first and
    /// end after it.
    Half(StepId, StepId),
    /// A section would be further in than [`DEEPEST`].
    Deep,
}

impl Misfit {
    /// As a [`BracketError`], naming the section other than `own` where
    /// two would be half over each other.
    fn about(self, own: StepId) -> BracketError {
        match self {
            Misfit::Backwards => BracketError::EndsBefore,
            Misfit::Half(outer, inner) if outer == own => BracketError::HalfOver(inner),
            Misfit::Half(outer, _) => BracketError::HalfOver(outer),
            Misfit::Deep => BracketError::TooDeep,
        }
    }
}

/// Where carried steps land: see [`Plan::carry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Place {
    /// Where among the rows that stay, as [`Plan::move_steps`] takes it:
    /// they go in before the row at this index once they are taken out
    /// (clamped to the end).
    pub index: usize,
    /// The section they land in: `None` for the plan's own level. Where
    /// a section ends at `index`, this says whether they go in at its
    /// end or after it ([`Plan::landings`] lists which can be). A
    /// section that holds nothing is landed in this way too.
    pub section: Option<StepId>,
}

/// What a carry does to the loops and drawn orders it meets: which key
/// is held while carrying. See [`Plan::carry`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Carry {
    /// No key: the steps go only where no loop or drawn order is
    /// crossed.
    #[default]
    Whole,
    /// Ctrl: the steps leave every loop and drawn order they cross, and
    /// take along the ones wholly among them.
    CutLoose,
    /// Alt: as [`CutLoose`](Carry::CutLoose), and the loops and drawn
    /// orders wholly among them are taken away and each of them waits
    /// for the operator.
    Stripped,
}

/// What a carry did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Carried {
    /// The steps carried, in their new order.
    pub steps: Vec<StepId>,
    /// The loops and drawn orders they were cut loose from.
    pub left: Vec<SpanId>,
    /// The loops and drawn orders no longer in the plan: taken away, or
    /// left with too few steps.
    pub gone: Vec<SpanId>,
}

/// Why steps can't be carried where they were asked to go.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CarryError {
    /// A step it names isn't in the plan, or it names none.
    #[error("that is no longer in the plan")]
    Missing,
    /// It names a section's heading: a section is moved with all it
    /// holds by [`Plan::move_group`].
    #[error("a section ({0}) moves with all it holds, not as a step")]
    Heading(StepId),
    /// The section asked for can't be landed in there.
    #[error("they can't land in that section there")]
    Nowhere,
    /// Carried whole, they would cross these loops and drawn orders.
    #[error("they would leave a loop or drawn order behind")]
    Crosses(Vec<SpanId>),
}

/// Where a span is: its first and last step, or `None` over no steps.
type Reach = Option<(usize, usize)>;

/// Whether two spans each hold a part of the other and not all of it.
fn half_over(a: Reach, b: Reach) -> bool {
    let (Some((a, b)), Some((c, d))) = (a, b) else {
        return false;
    };
    let apart = b < c || d < a;
    let within = (a <= c && d <= b) || (c <= a && b <= d);
    !apart && !within
}

/// What drawing a loop does to a flow: twice, unless it comes again
/// already.
fn come_again(flow: &mut Flow) {
    if flow.repeat == Repeat::Once {
        flow.repeat = Repeat::Times(2);
    }
}

/// A sound that plays with a step, over what the step shows: music under
/// a passage, or under the last slides of a deck.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepSound {
    /// The sound.
    pub resource: Resource,
    /// The slide it starts at, counted from 0: it plays from there on, and
    /// stops when the step goes back before it.
    #[serde(default, skip_serializing_if = "is_first")]
    pub from: usize,
}

fn is_first(slide: &usize) -> bool {
    *slide == 0
}

/// One entry of a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// Stable id.
    pub id: StepId,
    /// What the operator sees in the rundown.
    pub label: String,
    /// Private notes for the operator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// What happens when the step is reached.
    pub action: Action,
    /// How long the operator says the step takes, over any guess.
    #[serde(default, with = "seconds", skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Duration>,
    /// Go on to the next step by itself when done: a video when it ends,
    /// anything else after its length. `Some(false)` waits for the
    /// operator; unset, it does as the section it is in, or the plan,
    /// says. On a section, it is what its steps do unless they say.
    /// See [`Plan::goes_on`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advance: Option<bool>,
    /// How many times the step comes, by itself. A section's heading
    /// has none: what a section holds comes again, or in a drawn order,
    /// by a [`Span`] attached to it. Set either with [`Plan::set_flow`].
    #[serde(default, skip_serializing_if = "Flow::is_usual")]
    pub flow: Flow,
    /// A sound that plays with the step. Only a step that shows something
    /// still (slides, a passage, a picture, pages) plays one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<StepSound>,
    /// What this step changes of the look it is given, or for a section
    /// what it changes for everything in it. See [`Plan::look_at`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<LookChange>,
    /// It was put here to fill a section, not picked by hand. An app
    /// that keeps what the operator usually uses leaves it out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub filled: bool,
}

impl Step {
    /// Whether this step starts a section rather than doing something.
    pub fn is_section(&self) -> bool {
        matches!(self.action, Action::Section { .. })
    }

    /// How far in this section sits; `None` for a step that isn't one.
    pub fn depth(&self) -> Option<u8> {
        match self.action {
            Action::Section { depth, .. } => Some(depth),
            _ => None,
        }
    }

    /// The last row this section was given to hold, for one that ends
    /// before the next section; `None` for a step that isn't one.
    pub fn last(&self) -> Option<StepId> {
        match self.action {
            Action::Section { last, .. } => last,
            _ => None,
        }
    }

    /// The time of day this section should be over by; `None` for a step
    /// that isn't one.
    pub fn done_by(&self) -> Option<TimeOfDay> {
        match self.action {
            Action::Section { done_by, .. } => done_by,
            _ => None,
        }
    }

    /// Gives this section its last row, or none; nothing for a step.
    fn set_last(&mut self, to: Option<StepId>) {
        if let Action::Section { last, .. } = &mut self.action {
            *last = to;
        }
    }

    /// Sets how far in this section sits; nothing for a step.
    fn set_depth(&mut self, to: u8) {
        if let Action::Section { depth, .. } = &mut self.action {
            *depth = to;
        }
    }
}

/// Where a section is: up to which row it holds, and how it ends there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bound {
    /// The row after the last it holds.
    end: usize,
    /// It ends at a last row given to it or to a section around it, not
    /// at a heading or the plan's end: what is put at its end isn't in it.
    shut: bool,
}

/// Where each row's group ends: for a section's heading, what it holds;
/// for a step, the row after it. Sections close at the next heading no
/// further in than they are, at their own last row, or where a section
/// around them closes.
fn bounds(steps: &[Step]) -> Vec<Bound> {
    let n = steps.len();
    let mut found: Vec<Bound> = (1..=n).map(|end| Bound { end, shut: false }).collect();
    let position = |id: StepId| steps.iter().position(|step| step.id == id);
    // Each open section: its heading, depth and the row after its last.
    let mut open: Vec<(usize, u8, Option<usize>)> = Vec::new();
    for at in 0..=n {
        let depth = steps.get(at).and_then(Step::depth);
        let closes = |(_, of, end): &(usize, u8, Option<usize>)| {
            at == n || *end == Some(at) || depth.is_some_and(|depth| depth <= *of)
        };
        if let Some(from) = open.iter().position(closes) {
            let mut shut = false;
            for (heading, _, end) in open.drain(from..) {
                shut |= end == Some(at);
                found[heading] = Bound { end: at, shut };
            }
        }
        if let Some(depth) = depth {
            let end = steps[at]
                .last()
                .and_then(position)
                .filter(|&last| last >= at)
                .map(|last| last + 1);
            open.push((at, depth, end));
        }
    }
    found
}

/// An ordered list of steps, prepared ahead of time. Saved as JSON.
///
/// It is one flow: the steps come one after another, each going on by
/// itself or waiting ([`Plan::goes_on`]). Sections name parts of it and
/// say nothing of how it moves. What comes again, or in a drawn order, is
/// a [`Span`] laid over some of the steps, or the plan's own
/// [`flow`](Plan::flow) for all of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// E.g. "Sunday service, March 3".
    pub title: String,
    steps: Vec<Step>,
    #[serde(default)]
    next_id: u64,
    /// The loops and drawn orders laid over the steps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    spans: Vec<Span>,
    #[serde(default, skip_serializing_if = "is_none_yet")]
    next_span: u64,
    /// Notes for the operator, on what they are about.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) notes: Vec<crate::note::Note>,
    #[serde(default, skip_serializing_if = "is_none_yet")]
    pub(crate) next_note: u64,
    /// How the stage looks while this plan runs; `None` uses the app's
    /// default look. Saved with the plan, so it looks the same anywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<Look>,
    /// An order of service to start new plans from, such as "Domingo".
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub template: bool,
    /// Folders of media only this plan needs, as typed (`~/Church/Campaña`).
    /// An app indexes them while the plan is open, beside its own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<PathBuf>,
    /// The Bibles this plan searches, by name (`RV1909`), out of the ones
    /// an app has. Empty searches them all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bibles: Vec<String>,
    /// When the service starts, such as 10:30. With it an app can say at
    /// what time each section comes, and how far behind the service runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starts: Option<TimeOfDay>,
    /// How long the whole service is meant to take. An app sets what is
    /// planned against it, as a section's steps against its time.
    #[serde(default, with = "seconds", skip_serializing_if = "Option::is_none")]
    pub target: Option<Duration>,
    /// Things kept ready outside the running order: a spare song, a notice
    /// to move a car. Shown when wanted, without moving the plan.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub standby: Vec<Resource>,
    /// Whether steps go on by themselves unless a section or a step says
    /// otherwise. See [`Plan::goes_on`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub advance: bool,
    /// How many times the plan comes, and in what order its steps do.
    #[serde(default, skip_serializing_if = "Flow::is_usual")]
    pub flow: Flow,
}

impl Default for Plan {
    /// An empty plan titled "Untitled".
    fn default() -> Self {
        Self::new("Untitled")
    }
}

impl Plan {
    /// An empty plan.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            steps: Vec::new(),
            next_id: 0,
            spans: Vec::new(),
            next_span: 0,
            notes: Vec::new(),
            next_note: 0,
            look: None,
            template: false,
            folders: Vec::new(),
            bibles: Vec::new(),
            starts: None,
            target: None,
            standby: Vec::new(),
            advance: false,
            flow: Flow::default(),
        }
    }

    /// All steps, in order.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Number of steps.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether there are no steps.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// A step by id.
    pub fn get(&self, id: StepId) -> Option<&Step> {
        self.steps.iter().find(|s| s.id == id)
    }

    /// A step by id, for editing its label, notes or action.
    pub fn get_mut(&mut self, id: StepId) -> Option<&mut Step> {
        self.steps.iter_mut().find(|s| s.id == id)
    }

    /// Where a step currently is.
    pub fn position(&self, id: StepId) -> Option<usize> {
        self.steps.iter().position(|s| s.id == id)
    }

    /// Appends a step.
    pub fn push(&mut self, label: impl Into<String>, action: Action) -> StepId {
        self.insert(self.steps.len(), label, action)
    }

    /// Inserts a step at `index` (clamped to the end).
    pub fn insert(&mut self, index: usize, label: impl Into<String>, action: Action) -> StepId {
        let step = self.new_step(label, action);
        let id = step.id;
        self.steps.insert(index.min(self.steps.len()), step);
        self.settle();
        id
    }

    /// Puts `resource` on standby, after what's there. False if it already
    /// is.
    pub fn keep_ready(&mut self, resource: Resource) -> bool {
        let new = !self.standby.iter().any(|r| r.id == resource.id);
        if new {
            self.standby.push(resource);
        }
        new
    }

    /// Appends a step showing `resource`, labeled with its title.
    pub fn show(&mut self, resource: Resource) -> StepId {
        let label = resource.title.clone();
        self.push(label, Action::Show { resource })
    }

    /// Appends a live slot.
    pub fn live(&mut self, label: impl Into<String>, hint: Option<Query>) -> StepId {
        self.push(
            label,
            Action::Live {
                hint,
                shown: Vec::new(),
            },
        )
    }

    /// Appends a section, which runs until the next one.
    pub fn section(&mut self, title: impl Into<String>, budget: Option<Duration>) -> StepId {
        let action = Action::Section {
            budget,
            depth: 0,
            last: None,
            done_by: None,
        };
        self.push(title, action)
    }

    /// Where what the section at `at` holds ends: the row after its last
    /// row where it was given one, else the next section no further in
    /// than it, and never past where the section around it ends; or the
    /// end of the plan. `at + 1` for a step that isn't a section.
    pub fn group_end(&self, at: usize) -> usize {
        match self.steps.get(at) {
            Some(step) if step.is_section() => bounds(&self.steps)[at].end,
            _ => (at + 1).min(self.steps.len()),
        }
    }

    /// Whether the section at `at` ends at a last row given to it or to
    /// a section around it, rather than at a heading or the plan's end:
    /// then what is put at its end isn't in it. False for a step.
    pub fn ends_shut(&self, at: usize) -> bool {
        match self.steps.get(at) {
            Some(step) if step.is_section() => bounds(&self.steps)[at].shut,
            _ => false,
        }
    }

    /// The last row the section `section` holds now: its own heading
    /// when it holds nothing. `None` if it isn't a section.
    pub fn last_of(&self, section: StepId) -> Option<StepId> {
        let at = self.position(section)?;
        if !self.steps[at].is_section() {
            return None;
        }
        Some(self.steps[self.group_end(at) - 1].id)
    }

    /// The section that holds the row at `index` most closely, by where
    /// it is in the plan, with every section's bounds: `None` at the
    /// plan's own level.
    fn holder_in(&self, bounds: &[Bound], index: usize) -> Option<usize> {
        (0..index.min(self.steps.len()))
            .rev()
            .find(|&at| self.steps[at].is_section() && bounds[at].end > index)
    }

    /// How far in the row at `index` is: a section at its depth, a step
    /// one further in than the section it is in.
    pub fn level(&self, index: usize) -> u8 {
        match self.steps.get(index).and_then(Step::depth) {
            Some(depth) => depth,
            None => self
                .holder_in(&bounds(&self.steps), index)
                .and_then(|at| self.steps[at].depth())
                .map_or(0, |depth| depth + 1),
        }
    }

    /// How far in something put at `at` would be, and the section that
    /// would hold it: a section holds what is put at its end too, unless
    /// it [ends shut](Plan::ends_shut) there.
    pub fn level_put_at(&self, at: usize) -> (u8, Option<StepId>) {
        let at = at.min(self.steps.len());
        let bounds = bounds(&self.steps);
        let holder = (0..at).rev().find(|&section| {
            let bound = bounds[section];
            self.steps[section].is_section() && (at < bound.end || (at == bound.end && !bound.shut))
        });
        match holder {
            Some(section) => (
                self.steps[section].depth().unwrap_or(0) + 1,
                Some(self.steps[section].id),
            ),
            None => (0, None),
        }
    }

    /// The sections the step at `index` is in, outermost first. A
    /// section is not in itself.
    pub fn within(&self, index: usize) -> Vec<StepId> {
        let bounds = bounds(&self.steps);
        let mut found = Vec::new();
        let mut at = index;
        while let Some(holder) = self.holder_in(&bounds, at) {
            found.push(self.steps[holder].id);
            at = holder;
        }
        found.reverse();
        found
    }

    /// Whether the step at `index` goes on by itself when done: what it
    /// says itself, or else the nearest section it is in that says, or
    /// else the plan.
    pub fn goes_on(&self, index: usize) -> bool {
        let own = self.steps.get(index).and_then(|step| step.advance);
        own.or_else(|| {
            self.within(index)
                .into_iter()
                .rev()
                .find_map(|id| self.get(id).and_then(|section| section.advance))
        })
        .unwrap_or(self.advance)
    }

    /// What the section `id` holds, as a plan of its own: its steps and
    /// the sections in it, with the flow of the span attached to it as
    /// the plan's and the way its steps go on as it stood in this plan.
    /// The spans wholly in it come along. For running a part of the plan
    /// by itself. `None` if `id` isn't a section.
    pub fn part(&self, id: StepId) -> Option<Plan> {
        let at = self.position(id)?;
        let depth = self.steps[at].depth()?;
        let mut part = self.clone();
        part.advance = self.goes_on(at);
        part.flow = self.flow_of(id).unwrap_or_default();
        part.steps = self.steps[at + 1..self.group_end(at)].to_vec();
        for step in &mut part.steps {
            if let Action::Section { depth: inner, .. } = &mut step.action {
                *inner = inner.saturating_sub(depth + 1);
            }
        }
        part.settle();
        Some(part)
    }

    /// The steps the span `id` is over, as a plan of its own, with the
    /// span's flow as the plan's: for telling how long once round it
    /// takes. The spans wholly in it come along. A section it holds only
    /// the start of has no time of its own there: its time is for all of
    /// it. `None` if there is no such span, or it is over no steps.
    pub fn span_part(&self, id: SpanId) -> Option<Plan> {
        let flow = self.span(id)?.flow;
        let (first, last) = self.span_range(id)?;
        let mut part = self.clone();
        part.flow = flow;
        part.spans.retain(|other| other.id != id);
        part.steps = self.steps[first..=last].to_vec();
        for (at, step) in part.steps.iter_mut().enumerate() {
            if let Action::Section { budget, .. } = &mut step.action
                && self.group_end(first + at) > last + 1
            {
                *budget = None;
            }
        }
        part.settle();
        Some(part)
    }

    /// The look of the step at `index`. Each level changes what it is
    /// given: `usual` is the look for all plans, the plan's own replaces
    /// it, then every section the step is in changes it, outermost first,
    /// and last the step itself.
    pub fn look_at(&self, index: usize, usual: &Look) -> Look {
        let mut look = self.look.clone().unwrap_or_else(|| usual.clone());
        let levels = self
            .within(index)
            .into_iter()
            .filter_map(|id| self.get(id))
            .chain(self.steps.get(index));
        for change in levels.filter_map(|step| step.look.as_ref()) {
            change.apply(&mut look);
        }
        look
    }

    /// Moves the section `id`, with every section it holds, one level
    /// in (`true`) or out. In, it becomes a part of the section before
    /// it; nothing goes further in than [`DEEPEST`], or in with no
    /// section before it to be a part of. Returns whether it moved.
    pub fn nest(&mut self, id: StepId, inward: bool) -> bool {
        let Some(at) = self.position(id) else {
            return false;
        };
        let Some(depth) = self.steps[at].depth() else {
            return false;
        };
        let end = self.group_end(at);
        let deepest = self.steps[at..end]
            .iter()
            .filter_map(Step::depth)
            .max()
            .unwrap_or(depth);
        let before = self.steps[..at].iter().rev().find_map(Step::depth);
        let allowed = match inward {
            true => deepest < DEEPEST && before.is_some_and(|before| depth <= before),
            false => depth > 0,
        };
        if !allowed {
            return false;
        }
        // In, the section before it at its level takes it in, though
        // that one was given an end before it.
        let joins = self.steps[..at]
            .iter()
            .rposition(|step| step.depth().is_some_and(|before| before <= depth))
            .filter(|&before| self.steps[before].depth() == Some(depth));
        if let Some(before) = joins.filter(|_| inward)
            && self.steps[before].last().is_some()
        {
            let last = self.steps[end - 1].id;
            self.steps[before].set_last(Some(last));
        }
        for step in &mut self.steps[at..end] {
            if let Action::Section { depth, .. } = &mut step.action {
                *depth = if inward { *depth + 1 } else { *depth - 1 };
            }
        }
        self.settle();
        true
    }

    /// Keeps every section and span where it can be. No section is more
    /// than one level further in than the section that holds it, and the
    /// plan's own are at the plan's level. A span that comes once, in
    /// the plan's order, is nothing and is gone, as is one whose steps
    /// or section are gone. A drawn order a heading has come into is the
    /// plan's order again. Of two spans left half over each other, the
    /// newer goes. Moving or removing steps can leave any of these.
    fn settle(&mut self) {
        // Each section's depth, outermost first: the sections still open
        // where it is are those around it.
        let mut open: Vec<(u8, Option<usize>)> = Vec::new();
        for at in 0..self.steps.len() {
            open.retain(|(_, end)| end.is_none_or(|end| end > at));
            let Some(depth) = self.steps[at].depth() else {
                continue;
            };
            let close = |open: &mut Vec<(u8, Option<usize>)>, depth: u8| {
                if let Some(from) = open.iter().position(|(of, _)| depth <= *of) {
                    open.truncate(from);
                }
            };
            close(&mut open, depth);
            let most = open.last().map_or(0, |(holder, _)| holder + 1).min(DEEPEST);
            let depth = depth.min(most);
            close(&mut open, depth);
            self.steps[at].set_depth(depth);
            // Where it ends by its own last row, if that is after it.
            let own = self.steps[at]
                .last()
                .and_then(|last| self.position(last))
                .filter(|&last| last >= at)
                .map(|last| last + 1);
            // It ends where a section around it ends, at the latest.
            let around = open.iter().filter_map(|(_, end)| *end).min();
            let end = match (own, around) {
                (Some(own), Some(around)) => Some(own.min(around)),
                (own, around) => own.or(around),
            };
            open.push((depth, end));
        }
        // A last row it no longer holds is none: one moved before its
        // heading, or past where it ends by a heading or what is around.
        let bounds = bounds(&self.steps);
        for (at, bound) in bounds.into_iter().enumerate() {
            let Some(last) = self.steps[at].last() else {
                continue;
            };
            let held = self
                .position(last)
                .is_some_and(|last| last >= at && last < bound.end);
            if !held {
                self.steps[at].set_last(None);
            }
        }
        let mut spans = std::mem::take(&mut self.spans);
        spans.sort_by_key(|span| span.id);
        let mut kept: Vec<(Span, Reach)> = Vec::new();
        for mut span in spans {
            // Its two steps in the plan's order, whichever way they moved.
            if let Over::Steps { first, last } = span.over
                && self.position(first) > self.position(last)
            {
                span.over = Over::Steps {
                    first: last,
                    last: first,
                };
            }
            let Ok(reach) = self.reach(span.over) else {
                continue;
            };
            if reach.is_some_and(|reach| self.heading_among(reach)) {
                span.flow.shuffle = false;
            }
            if !span.flow.is_usual() && kept.iter().all(|(_, other)| !half_over(reach, *other)) {
                kept.push((span, reach));
            }
        }
        self.spans = kept.into_iter().map(|(span, _)| span).collect();
        // And a note that was on what is gone is on what is left.
        self.keep_notes();
    }

    /// Brings a plan saved before there were spans to how it is kept
    /// now. `ended` is each section that ended at a step, with that step
    /// ([`ends_before_spans`]). Such a section with no name was a loop
    /// drawn round some steps: it becomes a span from the first step it
    /// held to the one it ended at, and its row is gone. Any other
    /// section that came again, or in a drawn order, hands that to a
    /// span attached to it. A plan saved since has neither.
    fn upgrade_before_spans(&mut self, ended: &[(StepId, StepId)]) {
        let mut found = Vec::new();
        let mut gone = Vec::new();
        for (at, step) in self.steps.iter().enumerate() {
            if !step.is_section() {
                continue;
            }
            let last = ended.iter().find(|(section, _)| *section == step.id);
            let over = match last {
                Some((_, last)) if step.label.is_empty() => {
                    gone.push(step.id);
                    let first = self.steps[at + 1..].iter().find(|step| !step.is_section());
                    let Some(first) = first.filter(|_| self.position(*last) > Some(at)) else {
                        continue;
                    };
                    Over::Steps {
                        first: first.id,
                        last: *last,
                    }
                }
                _ => Over::Section(step.id),
            };
            found.push((over, step.flow));
        }
        self.steps.retain(|step| !gone.contains(&step.id));
        for step in self.steps.iter_mut().filter(|step| step.is_section()) {
            step.flow = Flow::default();
        }
        for (over, flow) in found {
            if !flow.is_usual() {
                self.spans.push(Span {
                    id: SpanId(self.next_span),
                    over,
                    flow,
                });
                self.next_span += 1;
            }
        }
    }

    /// Where `over` is in the plan now: its first and last step. `None`
    /// for a section that holds no step yet. An error if what it names
    /// is gone, or is not what it should be.
    fn reach(&self, over: Over) -> Result<Reach, SpanError> {
        let step = |at: &usize| !self.steps[*at].is_section();
        match over {
            Over::Steps { first, last } => {
                let first = self.position(first).ok_or(SpanError::Missing)?;
                let last = self.position(last).ok_or(SpanError::Missing)?;
                if first > last || !step(&first) || !step(&last) {
                    return Err(SpanError::Missing);
                }
                Ok(Some((first, last)))
            }
            Over::Section(id) => {
                let at = self.position(id).ok_or(SpanError::Missing)?;
                if step(&at) {
                    return Err(SpanError::Missing);
                }
                let mut held = at + 1..self.group_end(at);
                Ok(held.clone().find(step).zip(held.rfind(step)))
            }
        }
    }

    /// Whether a section's heading or a section's end is among the rows
    /// from one step to another: they are not all in one section.
    fn heading_among(&self, (first, last): (usize, usize)) -> bool {
        self.steps[first..=last].iter().any(Step::is_section)
            || self.within(first) != self.within(last)
    }

    /// Whether a span over `over` that moves as `flow` says can be in
    /// the plan, beside every span there but `but`.
    fn fits(&self, over: Over, flow: Flow, but: Option<SpanId>) -> Result<(), SpanError> {
        let reach = self.reach(over)?;
        if flow.shuffle && reach.is_some_and(|reach| self.heading_among(reach)) {
            return Err(SpanError::AcrossHeading);
        }
        let others = self.spans.iter().filter(|span| Some(span.id) != but);
        for other in others {
            if half_over(reach, self.reach(other.over).ok().flatten()) {
                return Err(SpanError::HalfOver(other.id));
            }
        }
        Ok(())
    }

    /// Every loop and drawn order laid over the steps.
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// A span by id.
    pub fn span(&self, id: SpanId) -> Option<&Span> {
        self.spans.iter().find(|span| span.id == id)
    }

    /// The span attached to the section `section`, if what it holds
    /// comes again or in a drawn order.
    pub fn section_span(&self, section: StepId) -> Option<&Span> {
        self.spans
            .iter()
            .find(|span| span.over == Over::Section(section))
    }

    /// Where the span `id` is now: its first and its last step. `None`
    /// if there is no such span, or it is attached to a section that
    /// holds no step yet.
    pub fn span_range(&self, id: SpanId) -> Option<(usize, usize)> {
        self.reach(self.span(id)?.over).ok().flatten()
    }

    /// Every span that is over some steps, with its first and its last
    /// step, outermost first: one that holds another comes before it,
    /// and otherwise they are in the plan's order.
    pub fn span_ranges(&self) -> Vec<(SpanId, usize, usize)> {
        let mut found: Vec<(Span, usize, usize)> = self
            .spans
            .iter()
            .filter_map(|span| {
                let (first, last) = self.reach(span.over).ok().flatten()?;
                Some((*span, first, last))
            })
            .collect();
        // Over the very same steps, a section's is outside one drawn
        // from step to step, and an outer section's outside an inner's.
        found.sort_by_key(|(span, first, last)| {
            let heading = match span.over {
                Over::Section(id) => self.position(id),
                Over::Steps { .. } => None,
            };
            (
                *first,
                std::cmp::Reverse(*last),
                heading.unwrap_or(usize::MAX),
                span.id,
            )
        });
        found
            .into_iter()
            .map(|(span, first, last)| (span.id, first, last))
            .collect()
    }

    /// Every span that is over some steps, with its first and its last
    /// step as they are now, outermost first as [`span_ranges`] has
    /// them. A span attached to a section that holds no step yet is left
    /// out.
    ///
    /// [`span_ranges`]: Plan::span_ranges
    pub fn laid_spans(&self) -> Vec<(Span, usize, usize)> {
        self.span_ranges()
            .into_iter()
            .filter_map(|(id, first, last)| Some((*self.span(id)?, first, last)))
            .collect()
    }

    /// The step the plan goes to after the row at `index` when nothing
    /// comes again: the next one that isn't a section's heading, across
    /// any headings. `None` after the last step.
    pub fn next_step(&self, index: usize) -> Option<usize> {
        let from = index.saturating_add(1);
        (from..self.steps.len()).find(|&at| !self.steps[at].is_section())
    }

    /// What [`loop_over`] would say, with the plan left as it is: the
    /// span it would make or change, or why it can't.
    ///
    /// [`loop_over`]: Plan::loop_over
    pub fn would_loop_over(&self, a: StepId, b: StepId) -> Result<SpanId, SpanError> {
        self.clone().loop_over(a, b)
    }

    /// What [`shuffle_over`] would say, with the plan left as it is.
    ///
    /// [`shuffle_over`]: Plan::shuffle_over
    pub fn would_shuffle_over(&self, a: StepId, b: StepId) -> Result<SpanId, SpanError> {
        self.clone().shuffle_over(a, b)
    }

    /// What [`loop_section`] would say, with the plan left as it is.
    ///
    /// [`loop_section`]: Plan::loop_section
    pub fn would_loop_section(&self, section: StepId) -> Result<SpanId, SpanError> {
        self.clone().loop_section(section)
    }

    /// What [`shuffle_section`] would say, with the plan left as it is.
    ///
    /// [`shuffle_section`]: Plan::shuffle_section
    pub fn would_shuffle_section(&self, section: StepId) -> Result<SpanId, SpanError> {
        self.clone().shuffle_section(section)
    }

    /// The spans the row at `index` is in, outermost first.
    pub fn spans_at(&self, index: usize) -> Vec<SpanId> {
        self.span_ranges()
            .into_iter()
            .filter(|(_, first, last)| (*first..=*last).contains(&index))
            .map(|(id, ..)| id)
            .collect()
    }

    /// Draws a loop round the steps from `a` to `b`, whichever comes
    /// first: they come twice, until that is changed. It may cross a
    /// section's heading: it is over the steps, not the sections. If a
    /// span is over just these steps already, it is that one that comes
    /// again. Returns the span.
    ///
    /// The limits every span keeps to: a loop goes round two steps or
    /// more and a drawn order three, unless it is attached to a section;
    /// a drawn order has no heading among its steps; and two spans are
    /// apart, or one is wholly in the other.
    pub fn loop_over(&mut self, a: StepId, b: StepId) -> Result<SpanId, SpanError> {
        self.lay(Over::Steps { first: a, last: b }, 2, come_again)
    }

    /// Has the steps from `a` to `b` come in an order drawn afresh each
    /// time, as [`loop_over`] has them come again. They are in one
    /// section, or all at the plan's own level.
    ///
    /// [`loop_over`]: Plan::loop_over
    pub fn shuffle_over(&mut self, a: StepId, b: StepId) -> Result<SpanId, SpanError> {
        self.lay(Over::Steps { first: a, last: b }, 3, |flow| {
            flow.shuffle = true;
        })
    }

    /// What [`loop_by_hand`] would do, with the plan left as it is.
    ///
    /// [`loop_by_hand`]: Plan::loop_by_hand
    pub fn would_loop_by_hand(&self, a: StepId, b: StepId) -> Result<Looped, SpanError> {
        self.clone().loop_by_hand(a, b)
    }

    /// Draws a loop round the steps from `a` to `b` as a hand does it
    /// along the plan. Where [`loop_over`] would refuse it for taking in
    /// a part of another loop drawn from step to step, the two become
    /// one loop round all of their steps, coming as often as the one
    /// that was there. A loop wholly inside another, or round it, is a
    /// loop of its own, as before.
    ///
    /// [`loop_over`]: Plan::loop_over
    pub fn loop_by_hand(&mut self, a: StepId, b: StepId) -> Result<Looped, SpanError> {
        let met = match self.would_loop_over(a, b) {
            Ok(_) => return self.loop_over(a, b).map(Looped::Made),
            Err(SpanError::HalfOver(met)) => met,
            Err(why) => return Err(why),
        };
        let other = *self.span(met).ok_or(SpanError::Missing)?;
        let ranged = matches!(other.over, Over::Steps { .. });
        let Some((from, to)) = self
            .reach(other.over)?
            .filter(|_| ranged && !other.is_drawn())
        else {
            return Err(SpanError::HalfOver(met));
        };
        let (a, b) = (
            self.position(a).ok_or(SpanError::Missing)?,
            self.position(b).ok_or(SpanError::Missing)?,
        );
        let (first, last) = (from.min(a).min(b), to.max(a).max(b));
        let mut next = self.clone();
        next.remove_span(met);
        let (first, last) = (next.steps[first].id, next.steps[last].id);
        let grown = next.loop_over(first, last)?;
        next.set_flow(grown, other.flow)?;
        *self = next;
        Ok(Looped::Grown(grown))
    }

    /// What [`rub_loop`] would do, with the plan left as it is.
    ///
    /// [`rub_loop`]: Plan::rub_loop
    pub fn would_rub_loop(&self, id: SpanId, a: StepId, b: StepId) -> Result<(), SpanError> {
        self.clone().rub_loop(id, a, b)
    }

    /// Rubs the steps from `a` to `b` out of the loop `id`, one drawn
    /// from step to step: what is left of it either side still comes
    /// again, as often, where two steps or more are left there. A drawn
    /// order the span has as well is left as it was.
    pub fn rub_loop(&mut self, id: SpanId, a: StepId, b: StepId) -> Result<(), SpanError> {
        let span = *self.span(id).ok_or(SpanError::Missing)?;
        let ranged = matches!(span.over, Over::Steps { .. });
        let Some((from, to)) = self.reach(span.over)?.filter(|_| ranged && span.is_loop()) else {
            return Err(SpanError::Missing);
        };
        let (a, b) = (
            self.position(a).ok_or(SpanError::Missing)?,
            self.position(b).ok_or(SpanError::Missing)?,
        );
        let (first, last) = (a.min(b).max(from), a.max(b).min(to));
        if first > last {
            return Err(SpanError::Missing);
        }
        let mut next = self.clone();
        next.unloop(id);
        let steps = |plan: &Plan, of: std::ops::Range<usize>| -> Vec<StepId> {
            of.filter(|at| !plan.steps[*at].is_section())
                .map(|at| plan.steps[at].id)
                .collect()
        };
        for left in [steps(&next, from..first), steps(&next, last + 1..to + 1)] {
            if let (Some(from), Some(to), true) = (left.first(), left.last(), left.len() >= 2) {
                let again = next.loop_over(*from, *to)?;
                let mut flow = next.flow_of(again).unwrap_or_default();
                flow.repeat = span.flow.repeat;
                flow.ends = span.flow.ends;
                next.set_flow(again, flow)?;
            }
        }
        *self = next;
        Ok(())
    }

    /// What [`draw_over`] would do, with the plan left as it is.
    ///
    /// [`draw_over`]: Plan::draw_over
    pub fn would_draw_over(&self, a: StepId, b: StepId) -> Result<Drawn, SpanError> {
        self.clone().draw_over(a, b)
    }

    /// Draws an order over the steps from `a` to `b` as a hand does it
    /// along the plan, which may meet orders drawn there already (those
    /// from one step to another; one attached to a section is left to
    /// [`shuffle_over`]'s rules):
    ///
    /// - wholly inside one, it rubs that part of it out: what is left
    ///   either side stays in a drawn order where three steps or more
    ///   are left, and what came again still does;
    /// - sharing a step with one or more, without being inside one, it
    ///   and they become one order over all of their steps;
    /// - otherwise it is an order of its own, as [`shuffle_over`] makes
    ///   one: two that only touch stay two, each drawn by itself.
    ///
    /// Nothing is changed where the result can't be (see [`loop_over`]).
    ///
    /// [`shuffle_over`]: Plan::shuffle_over
    /// [`loop_over`]: Plan::loop_over
    pub fn draw_over(&mut self, a: StepId, b: StepId) -> Result<Drawn, SpanError> {
        let (a, b) = (
            self.position(a).ok_or(SpanError::Missing)?,
            self.position(b).ok_or(SpanError::Missing)?,
        );
        let step = |plan: &Plan, at: &usize| !plan.steps[*at].is_section();
        let (first, last) = (a.min(b)..=a.max(b))
            .find(|at| step(self, at))
            .zip((a.min(b)..=a.max(b)).rfind(|at| step(self, at)))
            .ok_or(SpanError::TooFew)?;
        // The orders drawn from step to step that share a step with it.
        let met: Vec<(SpanId, usize, usize)> = self
            .spans
            .iter()
            .filter(|span| span.flow.shuffle && matches!(span.over, Over::Steps { .. }))
            .filter_map(|span| {
                let (from, to) = self.reach(span.over).ok().flatten()?;
                (from <= last && first <= to).then_some((span.id, from, to))
            })
            .collect();
        let id_at = |plan: &Plan, at: usize| plan.steps[at].id;
        let mut next = self.clone();
        let done = match met
            .iter()
            .find(|(_, from, to)| *from <= first && last <= *to)
        {
            // Inside one: that part of it is rubbed out.
            Some(&(inside, from, to)) => {
                next.unshuffle(inside);
                let steps = |plan: &Plan, of: std::ops::Range<usize>| {
                    of.filter(|at| step(plan, at)).collect::<Vec<usize>>()
                };
                for left in [steps(&next, from..first), steps(&next, last + 1..to + 1)] {
                    if let (Some(from), Some(to), true) =
                        (left.first(), left.last(), left.len() >= 3)
                    {
                        let (from, to) = (id_at(&next, *from), id_at(&next, *to));
                        next.shuffle_over(from, to)?;
                    }
                }
                Drawn::Rubbed
            }
            None if met.is_empty() => {
                let (first, last) = (id_at(&next, first), id_at(&next, last));
                Drawn::Made(next.shuffle_over(first, last)?)
            }
            // Meeting some: one order over all of them.
            None => {
                let first = met
                    .iter()
                    .map(|(_, from, _)| *from)
                    .min()
                    .unwrap_or(first)
                    .min(first);
                let last = met
                    .iter()
                    .map(|(_, _, to)| *to)
                    .max()
                    .unwrap_or(last)
                    .max(last);
                for (id, ..) in &met {
                    next.unshuffle(*id);
                }
                let (first, last) = (id_at(&next, first), id_at(&next, last));
                Drawn::Grown(next.shuffle_over(first, last)?)
            }
        };
        *self = next;
        Ok(done)
    }

    /// Has all the section `section` holds come again: twice, until that
    /// is changed. A step put into the section later is in the loop.
    /// Returns the span attached to the section.
    pub fn loop_section(&mut self, section: StepId) -> Result<SpanId, SpanError> {
        self.lay(Over::Section(section), 0, come_again)
    }

    /// Has all the section `section` holds come in an order drawn afresh
    /// each time. Not a section that holds sections with steps before
    /// them: a drawn order has no heading among its steps.
    pub fn shuffle_section(&mut self, section: StepId) -> Result<SpanId, SpanError> {
        self.lay(Over::Section(section), 0, |flow| flow.shuffle = true)
    }

    /// The span over `over`, made if there is none, with its flow
    /// changed by `set`. A span over the very same steps is the one
    /// changed, and it is attached to the section when a section is what
    /// was asked for. An error with fewer than `least` steps from one
    /// step to another, or where [`fits`] says it can't be.
    ///
    /// [`fits`]: Plan::fits
    fn lay(
        &mut self,
        over: Over,
        least: usize,
        set: impl FnOnce(&mut Flow),
    ) -> Result<SpanId, SpanError> {
        // From the first step among them to the last, in the plan's
        // order: neither end is a heading.
        let over = match over {
            Over::Steps { first, last } => {
                let a = self.position(first).ok_or(SpanError::Missing)?;
                let b = self.position(last).ok_or(SpanError::Missing)?;
                let mut steps = self.steps[a.min(b)..=a.max(b)]
                    .iter()
                    .filter(|step| !step.is_section());
                let first = steps.next().ok_or(SpanError::TooFew)?;
                if steps.clone().count() + 1 < least {
                    return Err(SpanError::TooFew);
                }
                Over::Steps {
                    first: first.id,
                    last: steps.next_back().unwrap_or(first).id,
                }
            }
            Over::Section(_) => over,
        };
        let reach = self.reach(over)?;
        let ranged = matches!(over, Over::Steps { .. });
        let same = self
            .spans
            .iter()
            .position(|span| span.over == over)
            .or_else(|| {
                self.spans.iter().position(|span| {
                    (ranged || matches!(span.over, Over::Steps { .. }))
                        && reach.is_some()
                        && self.reach(span.over) == Ok(reach)
                })
            });
        let mut flow = same.map(|at| self.spans[at].flow).unwrap_or_default();
        set(&mut flow);
        self.fits(over, flow, same.map(|at| self.spans[at].id))?;
        let id = match same {
            Some(at) => {
                let span = &mut self.spans[at];
                span.flow = flow;
                if !ranged {
                    span.over = over;
                }
                span.id
            }
            None => {
                let id = SpanId(self.next_span);
                self.next_span += 1;
                self.spans.push(Span { id, over, flow });
                id
            }
        };
        self.settle();
        Ok(id)
    }

    /// How `of` moves: the plan, a step by itself, or a span. A
    /// section's heading says how the span attached to the section does,
    /// which with no such span is once, in the plan's order. `None` if
    /// there is no such step or span.
    pub fn flow_of(&self, of: impl Into<FlowOf>) -> Option<Flow> {
        match of.into() {
            FlowOf::Plan => Some(self.flow),
            FlowOf::Span(id) => self.span(id).map(|span| span.flow),
            FlowOf::Step(id) => {
                let step = self.get(id)?;
                Some(match step.is_section() {
                    true => self
                        .section_span(id)
                        .map(|span| span.flow)
                        .unwrap_or_default(),
                    false => step.flow,
                })
            }
        }
    }

    /// Has `of` move as `flow` says: the plan, a step by itself, or a
    /// span. For a section's heading it is the span attached to the
    /// section, made if there is none. A span that would come once, in
    /// the plan's order, is taken away. An error where a span can't be
    /// so (see [`loop_over`]), with nothing changed.
    ///
    /// [`loop_over`]: Plan::loop_over
    pub fn set_flow(&mut self, of: impl Into<FlowOf>, flow: Flow) -> Result<(), SpanError> {
        let (over, id) = match of.into() {
            FlowOf::Plan => {
                self.flow = flow;
                return Ok(());
            }
            FlowOf::Span(id) => (self.span(id).ok_or(SpanError::Missing)?.over, Some(id)),
            FlowOf::Step(id) => {
                let step = self.get_mut(id).ok_or(SpanError::Missing)?;
                if !step.is_section() {
                    step.flow = flow;
                    return Ok(());
                }
                let span = self.section_span(id).map(|span| span.id);
                (Over::Section(id), span)
            }
        };
        let was = id.and_then(|id| self.span(id)).map(|span| span.flow);
        // A drawn order newly asked for needs its three steps.
        if flow.shuffle
            && !was.is_some_and(|was| was.shuffle)
            && matches!(over, Over::Steps { .. })
            && self.reach(over)?.is_some_and(|(first, last)| {
                let steps = self.steps[first..=last].iter();
                steps.filter(|step| !step.is_section()).count() < 3
            })
        {
            return Err(SpanError::TooFew);
        }
        self.fits(over, flow, id)?;
        match id.and_then(|id| self.spans.iter_mut().find(|span| span.id == id)) {
            Some(span) => span.flow = flow,
            None => {
                self.spans.push(Span {
                    id: SpanId(self.next_span),
                    over,
                    flow,
                });
                self.next_span += 1;
            }
        }
        self.settle();
        Ok(())
    }

    /// Copies `ids` (steps; a section with all it holds), putting the
    /// copies right after the last row copied, in the same order and as
    /// far in. A loop or a drawn order wholly among what is copied is
    /// copied with it, onto the copies. A section that ended at the last
    /// row copied ends at its copy, so the copies stay in it. Returns the
    /// copies of `ids`, in the plan's order; empty where none is in the
    /// plan.
    pub fn duplicate(&mut self, ids: &[StepId]) -> Vec<StepId> {
        let mut rows: Vec<usize> = Vec::new();
        for id in ids {
            let Some(at) = self.position(*id) else {
                continue;
            };
            let end = match self.steps[at].is_section() {
                true => self.group_end(at),
                false => at + 1,
            };
            for row in at..end {
                if !rows.contains(&row) {
                    rows.push(row);
                }
            }
        }
        rows.sort_unstable();
        let Some(&anchor) = rows.last() else {
            return Vec::new();
        };
        let anchor_id = self.steps[anchor].id;
        // The copies, with new ids.
        let mut made: Vec<(StepId, StepId)> = Vec::new();
        let mut copies: Vec<Step> = Vec::new();
        for row in &rows {
            let mut step = self.steps[*row].clone();
            let new = StepId(self.next_id);
            self.next_id += 1;
            made.push((step.id, new));
            step.id = new;
            copies.push(step);
        }
        let copy_of = |id: StepId| made.iter().find(|(was, _)| *was == id).map(|(_, new)| *new);
        // A copied section ends at the copy of where it ended.
        for step in &mut copies {
            if let Action::Section {
                last: Some(last), ..
            } = &mut step.action
                && let Some(new) = copy_of(*last)
            {
                *last = new;
            }
        }
        // A section round them that ended at the last row copied ends at
        // its copy now.
        let anchor_copy = copy_of(anchor_id);
        for step in &mut self.steps {
            if let Action::Section { last, .. } = &mut step.action
                && *last == Some(anchor_id)
                && !made.iter().any(|(was, _)| *was == step.id)
            {
                *last = anchor_copy;
            }
        }
        self.steps.splice(anchor + 1..anchor + 1, copies);
        // The loops and drawn orders wholly among them, onto the copies.
        let mut spans: Vec<Span> = Vec::new();
        for span in &self.spans {
            let over = match span.over {
                Over::Steps { first, last } => match (copy_of(first), copy_of(last)) {
                    (Some(first), Some(last)) => Over::Steps { first, last },
                    _ => continue,
                },
                Over::Section(id) => match copy_of(id) {
                    Some(id) => Over::Section(id),
                    None => continue,
                },
            };
            let id = SpanId(self.next_span);
            self.next_span += 1;
            spans.push(Span {
                id,
                over,
                flow: span.flow,
            });
        }
        self.spans.extend(spans);
        self.settle();
        let mut found: Vec<StepId> = ids.iter().filter_map(|id| copy_of(*id)).collect();
        found.sort_by_key(|id| self.position(*id));
        found
    }

    /// Takes the span `id` away: its steps come once, in the plan's
    /// order. Returns it, if it was there.
    pub fn remove_span(&mut self, id: SpanId) -> Option<Span> {
        let at = self.spans.iter().position(|span| span.id == id)?;
        let span = self.spans.remove(at);
        self.keep_notes();
        Some(span)
    }

    /// Takes away that `of` comes again: it comes once. A span is gone
    /// unless it is there for the order of its steps too. Returns
    /// whether it came again.
    pub fn unloop(&mut self, of: impl Into<FlowOf>) -> bool {
        let of = of.into();
        let Some(mut flow) = self.flow_of(of) else {
            return false;
        };
        let was = flow.repeat != Repeat::Once;
        flow.repeat = Repeat::Once;
        flow.ends = Ending::default();
        self.set_flow(of, flow).is_ok() && was
    }

    /// Takes away the drawn order of `of`: its steps come in the plan's
    /// order. A span is gone unless it comes again too. Returns whether
    /// it had one.
    pub fn unshuffle(&mut self, of: impl Into<FlowOf>) -> bool {
        let of = of.into();
        let Some(mut flow) = self.flow_of(of) else {
            return false;
        };
        let was = flow.shuffle;
        flow.shuffle = false;
        self.set_flow(of, flow).is_ok() && was
    }

    /// When the steps `leaving` go from where they are, a span drawn
    /// from or to one of them is drawn to the nearest of its steps that
    /// stays. With none that stays, it goes with them.
    fn let_go(&mut self, leaving: &[StepId]) {
        for at in 0..self.spans.len() {
            let Over::Steps { first, last } = self.spans[at].over else {
                continue;
            };
            if !leaving.contains(&first) && !leaving.contains(&last) {
                continue;
            }
            let Ok(Some((from, to))) = self.reach(self.spans[at].over) else {
                continue;
            };
            let mut staying = self.steps[from..=to]
                .iter()
                .filter(|step| !step.is_section() && !leaving.contains(&step.id));
            if let Some(first) = staying.next() {
                self.spans[at].over = Over::Steps {
                    first: first.id,
                    last: staying.next_back().unwrap_or(first).id,
                };
            }
        }
    }

    /// When the rows `leaving` go from where they are, a section whose
    /// last row is one of them ends at the nearest row before it that
    /// stays: its own heading, at the least.
    fn end_before(&mut self, leaving: &[StepId]) {
        for at in 0..self.steps.len() {
            let Some(last) = self.steps[at].last().filter(|last| leaving.contains(last)) else {
                continue;
            };
            if leaving.contains(&self.steps[at].id) {
                continue;
            }
            let Some(last) = self.position(last).filter(|&last| last >= at) else {
                continue;
            };
            let stays = self.steps[at..last]
                .iter()
                .rev()
                .find(|step| !leaving.contains(&step.id))
                .map(|step| step.id);
            self.steps[at].set_last(stays);
        }
    }

    /// Removes a step. A section's own steps stay, in the section before
    /// it where that one ran on to its heading, and otherwise in what
    /// was around it; the sections it held move out one level, to where
    /// it was. A span attached to it is gone.
    pub fn remove(&mut self, id: StepId) -> Option<Step> {
        self.let_go(&[id]);
        self.end_before(&[id]);
        let index = self.position(id)?;
        let Some(depth) = self.steps[index].depth() else {
            let step = self.steps.remove(index);
            self.settle();
            return Some(step);
        };
        let bounds = bounds(&self.steps);
        let end = bounds[index].end;
        let mut lasts = self.lasts();
        // The section before it at its level, that ran on to its heading,
        // takes its own steps up to the first section in it.
        let before = (0..index).rev().find(|&at| {
            self.steps[at].depth() == Some(depth) && bounds[at].end == index && !bounds[at].shut
        });
        let own = self.steps[index + 1..end]
            .iter()
            .take_while(|step| !step.is_section())
            .last()
            .map(|step| step.id);
        if let Some((before, own)) = before.zip(own) {
            let before = self.steps[before].id;
            for (section, last) in &mut lasts {
                if *section == before {
                    *last = own;
                }
            }
        }
        Some(self.unhead(index, lasts))
    }

    /// Takes the heading at `index` out, every other section holding
    /// what `lasts` says, or what it held where that can't be. What
    /// ended at the heading ends at the row before it.
    fn unhead(&mut self, index: usize, mut lasts: Vec<(StepId, StepId)>) -> Step {
        let id = self.steps[index].id;
        lasts.retain(|(section, _)| *section != id);
        if let Some(before) = index.checked_sub(1).map(|at| self.steps[at].id) {
            for (_, last) in &mut lasts {
                if *last == id {
                    *last = before;
                }
            }
        }
        let step = self.steps.remove(index);
        if self.shape(&lasts).is_err() {
            self.settle();
        }
        step
    }

    /// Each section's heading with the last row it holds now: its own
    /// heading for one that holds nothing.
    fn lasts(&self) -> Vec<(StepId, StepId)> {
        let bounds = bounds(&self.steps);
        self.steps
            .iter()
            .enumerate()
            .filter(|(_, step)| step.is_section())
            .map(|(at, step)| (step.id, self.steps[bounds[at].end - 1].id))
            .collect()
    }

    /// Lays the sections out as `lasts` says: each holds from its
    /// heading to the row given with it (a section not named holds
    /// nothing). Each is as far in as there are sections around it, and
    /// keeps a last row of its own where it had one, or where it would
    /// otherwise run on past that row. Then all is settled. Nothing is
    /// changed where two would be half over each other, or a section
    /// would be further in than [`DEEPEST`].
    fn shape(&mut self, lasts: &[(StepId, StepId)]) -> Result<(), Misfit> {
        let n = self.steps.len();
        let position: HashMap<StepId, usize> = self
            .steps
            .iter()
            .enumerate()
            .map(|(at, step)| (step.id, at))
            .collect();
        // Each section's heading and last row, in the plan's order.
        let mut laid: Vec<(usize, usize)> = Vec::new();
        for (at, step) in self.steps.iter().enumerate() {
            if !step.is_section() {
                continue;
            }
            let last = lasts
                .iter()
                .find(|(section, _)| *section == step.id)
                .and_then(|(_, last)| position.get(last).copied())
                .unwrap_or(at);
            if last < at {
                return Err(Misfit::Backwards);
            }
            laid.push((at, last));
        }
        // How far in each is, and the one around it.
        let mut depths: Vec<u8> = Vec::with_capacity(laid.len());
        let mut around: Vec<Option<usize>> = Vec::with_capacity(laid.len());
        let mut open: Vec<usize> = Vec::new();
        for (k, &(at, last)) in laid.iter().enumerate() {
            while open.last().is_some_and(|&top| laid[top].1 < at) {
                open.pop();
            }
            if let Some(&top) = open.last()
                && last > laid[top].1
            {
                let (outer, inner) = (self.steps[laid[top].0].id, self.steps[at].id);
                return Err(Misfit::Half(outer, inner));
            }
            let depth = u8::try_from(open.len()).unwrap_or(u8::MAX);
            if depth > DEEPEST {
                return Err(Misfit::Deep);
            }
            depths.push(depth);
            around.push(open.last().copied());
            open.push(k);
        }
        for (k, &(at, last)) in laid.iter().enumerate() {
            // Where it would end with no last row of its own.
            let next = laid[k + 1..]
                .iter()
                .zip(&depths[k + 1..])
                .find(|(_, depth)| **depth <= depths[k])
                .map_or(n, |((heading, _), _)| *heading);
            let outer = around[k].map_or(n, |outer| laid[outer].1 + 1);
            let runs_to = next.min(outer);
            let id = self.steps[last].id;
            let step = &mut self.steps[at];
            let given = step.last().is_some() || last + 1 < runs_to;
            step.set_depth(depths[k]);
            step.set_last(given.then_some(id));
        }
        self.settle();
        Ok(())
    }

    /// Moves a step to `index` in the plan without it (clamped to the
    /// end). Returns whether it exists.
    pub fn move_to(&mut self, id: StepId, index: usize) -> bool {
        self.move_steps(&[id], index)
    }

    /// What moving `id` takes along: the step alone, or a section with
    /// everything it holds, sections in it too. Empty if there's no such
    /// step.
    pub fn carried(&self, id: StepId) -> Vec<StepId> {
        let Some(at) = self.position(id) else {
            return Vec::new();
        };
        self.steps[at..self.group_end(at)]
            .iter()
            .map(|step| step.id)
            .collect()
    }

    /// Moves the steps `ids`, in their plan order, to `index` in the plan
    /// without them (clamped to the end). Returns whether any exist.
    ///
    /// A span is over what is between its two steps, so a step moved in
    /// between them is in it, and one moved out is not. A span whose
    /// steps all move goes with them.
    pub fn move_steps(&mut self, ids: &[StepId], index: usize) -> bool {
        let moved = self.place(ids, index);
        self.settle();
        moved
    }

    /// Moves the section `ids` begins with, and all it holds, to `index`
    /// in the plan without them, with the section at `depth`: the
    /// sections it holds go in or out as far as it does. Then every
    /// section and span is kept where it can be, as after any move.
    /// Returns whether any of `ids` exist.
    pub fn move_group(&mut self, ids: &[StepId], index: usize, depth: u8) -> bool {
        let own = ids
            .first()
            .and_then(|id| self.get(*id))
            .and_then(Step::depth);
        if let Some(own) = own {
            for step in self.steps.iter_mut().filter(|step| ids.contains(&step.id)) {
                if let Action::Section { depth: at, .. } = &mut step.action {
                    // In by as much as the first goes in, or out.
                    *at = (i16::from(*at) + i16::from(depth) - i16::from(own))
                        .clamp(0, i16::from(DEEPEST)) as u8;
                }
            }
        }
        let moved = self.place(ids, index);
        self.settle();
        moved
    }

    /// Puts `ids`, in their plan order, at `index` in the plan without
    /// them. Sections are left at whatever depth they have. Put where
    /// they already are, nothing changes.
    fn place(&mut self, ids: &[StepId], index: usize) -> bool {
        let before: Vec<StepId> = self.steps.iter().map(|step| step.id).collect();
        let (moving, mut after): (Vec<StepId>, Vec<StepId>) =
            before.iter().partition(|id| ids.contains(*id));
        let at = index.min(after.len());
        after.splice(at..at, moving.iter().copied());
        if after == before {
            return !moving.is_empty();
        }
        self.let_go(ids);
        self.end_before(ids);
        let (moving, mut rest): (Vec<Step>, Vec<Step>) = std::mem::take(&mut self.steps)
            .into_iter()
            .partition(|step| ids.contains(&step.id));
        rest.splice(at..at, moving);
        self.steps = rest;
        true
    }

    /// A step with a new id, not yet in the plan.
    fn new_step(&mut self, label: impl Into<String>, action: Action) -> Step {
        let id = StepId(self.next_id);
        self.next_id += 1;
        Step {
            id,
            label: label.into(),
            notes: None,
            action,
            estimate: None,
            advance: None,
            flow: Flow::default(),
            sound: None,
            look: None,
            filled: false,
        }
    }

    /// Whether every drawn order in this plan still is one in `next`;
    /// the first that isn't, if one isn't.
    fn cut_order(&self, next: &Plan) -> Option<SpanId> {
        self.spans
            .iter()
            .filter(|span| span.flow.shuffle)
            .find(|span| !next.span(span.id).is_some_and(|span| span.flow.shuffle))
            .map(|span| span.id)
    }

    /// What [`bracket_over`] would say, with the plan left as it is: the
    /// id the new section would have, or why it can't be.
    ///
    /// [`bracket_over`]: Plan::bracket_over
    pub fn would_bracket_over(&self, a: StepId, b: StepId) -> Result<StepId, BracketError> {
        self.clone().bracket_over(a, b, "")
    }

    /// Puts a new section named `label` round the rows from `a` to `b`,
    /// whichever comes first: its heading goes in before the first, and
    /// it ends at the last. Ending on a section's heading, it takes in
    /// all that section holds. It is in whatever section those rows are
    /// in, and around the sections wholly among them, which go one level
    /// further in. Returns the new section's id.
    ///
    /// Refused, with nothing changed, where it would take in a part of
    /// a section ([`BracketError::HalfOver`]), put a section further in
    /// than [`DEEPEST`], or end a drawn order by coming among its steps.
    pub fn bracket_over(
        &mut self,
        a: StepId,
        b: StepId,
        label: impl Into<String>,
    ) -> Result<StepId, BracketError> {
        let a = self.position(a).ok_or(BracketError::Missing)?;
        let b = self.position(b).ok_or(BracketError::Missing)?;
        let (first, mut last) = (a.min(b), a.max(b));
        if self.steps[last].is_section() {
            last = self.group_end(last) - 1;
        }
        let mut next = self.clone();
        let mut lasts = next.lasts();
        let action = Action::Section {
            budget: None,
            depth: 0,
            last: Some(self.steps[last].id),
            done_by: None,
        };
        let heading = next.new_step(label, action);
        let id = heading.id;
        lasts.push((id, self.steps[last].id));
        next.steps.insert(first, heading);
        next.shape(&lasts).map_err(|misfit| misfit.about(id))?;
        if let Some(cut) = self.cut_order(&next) {
            return Err(BracketError::Cuts(cut));
        }
        *self = next;
        Ok(id)
    }

    /// Takes the section `section` away and leaves its steps where they
    /// are, in whatever section was around it; the sections it held go
    /// one level out. A loop or drawn order attached to it is laid over
    /// those steps instead. False if it isn't a section.
    pub fn unbracket(&mut self, section: StepId) -> bool {
        let Some(at) = self.position(section) else {
            return false;
        };
        if !self.steps[at].is_section() {
            return false;
        }
        let mut next = self.clone();
        if let Some(span) = next.section_span(section).map(|span| span.id) {
            let reach = next.reach(Over::Section(section)).ok().flatten();
            match reach {
                Some((first, last)) => {
                    let over = Over::Steps {
                        first: next.steps[first].id,
                        last: next.steps[last].id,
                    };
                    if let Some(span) = next.spans.iter_mut().find(|other| other.id == span) {
                        span.over = over;
                    }
                }
                None => {
                    next.remove_span(span);
                }
            }
        }
        let lasts = next.lasts();
        next.unhead(at, lasts);
        *self = next;
        true
    }

    /// What [`reach_section`] would say, with the plan left as it is.
    ///
    /// [`reach_section`]: Plan::reach_section
    pub fn would_reach_section(
        &self,
        section: StepId,
        last: Option<StepId>,
    ) -> Result<(), BracketError> {
        self.clone().reach_section(section, last)
    }

    /// Has the section `section` end at the row `last`, by hand: what
    /// comes after it is back in whatever holds the section. `last` may
    /// be its own heading, for a section that holds nothing, and on
    /// another section's heading it takes in all that one holds. With
    /// `None` it runs on to the next section no further in than itself,
    /// or until what holds it ends. Sections it comes to hold wholly go
    /// one level further in, and ones it no longer holds one out.
    ///
    /// Refused, with nothing changed, as [`bracket_over`] is, or where
    /// `last` comes before its heading.
    ///
    /// [`bracket_over`]: Plan::bracket_over
    pub fn reach_section(
        &mut self,
        section: StepId,
        last: Option<StepId>,
    ) -> Result<(), BracketError> {
        let at = self.position(section).ok_or(BracketError::Missing)?;
        if !self.steps[at].is_section() {
            return Err(BracketError::Missing);
        }
        let mut next = self.clone();
        match last {
            None => {
                next.steps[at].set_last(None);
                next.settle();
            }
            Some(last) => {
                let mut to = self.position(last).ok_or(BracketError::Missing)?;
                if to < at {
                    return Err(BracketError::EndsBefore);
                }
                if to > at && self.steps[to].is_section() {
                    to = self.group_end(to) - 1;
                }
                let last = self.steps[to].id;
                let mut lasts = next.lasts();
                for (own, end) in &mut lasts {
                    if *own == section {
                        *end = last;
                    }
                }
                next.steps[at].set_last(Some(last));
                next.shape(&lasts).map_err(|misfit| misfit.about(section))?;
            }
        }
        if let Some(cut) = self.cut_order(&next) {
            return Err(BracketError::Cuts(cut));
        }
        *self = next;
        Ok(())
    }

    /// The sections that end at `index` in this plan, innermost first,
    /// and the one around `index` that runs on past it.
    fn ending_at(&self, index: usize) -> (Vec<StepId>, Option<StepId>) {
        let index = index.min(self.steps.len());
        let bounds = bounds(&self.steps);
        let ending = (0..index)
            .rev()
            .filter(|&at| self.steps[at].is_section() && bounds[at].end == index)
            .map(|at| self.steps[at].id)
            .collect();
        let around = self.holder_in(&bounds, index).map(|at| self.steps[at].id);
        (ending, around)
    }

    /// The plan with the rows `ids` taken out: sections that ended at
    /// one end at the row before it that stays, and spans drawn from or
    /// to one are drawn to the nearest that stays. Nothing is settled.
    fn without(&self, ids: &[StepId]) -> (Plan, Vec<Step>) {
        let mut rest = self.clone();
        rest.let_go(ids);
        rest.end_before(ids);
        let (taken, kept) = std::mem::take(&mut rest.steps)
            .into_iter()
            .partition(|step| ids.contains(&step.id));
        rest.steps = kept;
        (rest, taken)
    }

    /// The sections steps carried to `index` among the rows that stay
    /// (without `steps`) can land in, innermost first, for a
    /// [`Place`]: each section that ends there, landing at its end, and
    /// last the section around `index`, or `None` at the plan's own
    /// level.
    pub fn landings(&self, steps: &[StepId], index: usize) -> Vec<Option<StepId>> {
        let (rest, _) = self.without(steps);
        let (ending, around) = rest.ending_at(index);
        ending.into_iter().map(Some).chain([around]).collect()
    }

    /// Puts `rows` in at `to` in this plan, which they are not in: in
    /// the section it names, and in every section around that one that
    /// would otherwise end where they go in. A section among them holds
    /// nothing.
    fn lay_in(&mut self, rows: Vec<Step>, to: Place) -> Result<(), CarryError> {
        let index = to.index.min(self.steps.len());
        let (ending, around) = self.ending_at(index);
        let grows = match ending.iter().position(|id| Some(*id) == to.section) {
            Some(from) => &ending[from..],
            None if around == to.section => &[],
            None => return Err(CarryError::Nowhere),
        };
        let Some(last) = rows.last().map(|step| step.id) else {
            return Ok(());
        };
        let mut lasts = self.lasts();
        for (section, end) in &mut lasts {
            if grows.contains(section) {
                *end = last;
            }
        }
        lasts.extend(
            rows.iter()
                .filter(|step| step.is_section())
                .map(|step| (step.id, step.id)),
        );
        self.steps.splice(index..index, rows);
        self.shape(&lasts).map_err(|_| CarryError::Nowhere)
    }

    /// Puts a new step in at `to`, in the section it names: see
    /// [`Place`]. A new section put in so holds nothing. Returns its id,
    /// or with nothing changed, [`CarryError::Nowhere`] where that
    /// section can't be landed in there.
    pub fn insert_at(
        &mut self,
        to: Place,
        label: impl Into<String>,
        action: Action,
    ) -> Result<StepId, CarryError> {
        let mut next = self.clone();
        let step = next.new_step(label, action);
        let id = step.id;
        next.lay_in(vec![step], to)?;
        *self = next;
        Ok(id)
    }

    /// The steps the span `id` is over now, in the plan's order: what
    /// carrying it along its line takes. Carrying just these never
    /// crosses it. Empty if there is no such span.
    pub fn span_steps(&self, id: SpanId) -> Vec<StepId> {
        let Some((first, last)) = self.span_range(id) else {
            return Vec::new();
        };
        self.steps[first..=last]
            .iter()
            .filter(|step| !step.is_section())
            .map(|step| step.id)
            .collect()
    }

    /// What [`carry`] would do, with the plan left as it is.
    ///
    /// [`carry`]: Plan::carry
    pub fn would_carry(
        &self,
        steps: &[StepId],
        to: Place,
        how: Carry,
    ) -> Result<Carried, CarryError> {
        self.clone().carry(steps, to, how)
    }

    /// Carries the steps `steps`, a selection that need not be together,
    /// to `to`: they land together there, in the plan's order. A
    /// selection that names a section's heading is refused: a section
    /// with all it holds is moved by [`move_group`], which keeps to no
    /// span's rules, and `carry` is for the steps themselves.
    ///
    /// A loop or drawn order is crossed when some of its steps are
    /// carried and others are not, unless the carried ones land inside
    /// it again: moving about inside a loop crosses nothing. One
    /// attached to a section is crossed too when all its steps leave
    /// the section. One drawn from step to step wholly among the carried
    /// steps goes with them. By `how`:
    ///
    /// - [`Carry::Whole`]: refused if any is crossed, naming them all.
    /// - [`Carry::CutLoose`]: the steps leave each one crossed, which
    ///   keeps its other steps, and is a loop no more with one step left,
    ///   nor a drawn order with two.
    /// - [`Carry::Stripped`]: as `CutLoose`, and those wholly among them
    ///   are taken away and each step waits for the operator.
    ///
    /// Whether a step goes on by itself is its own, and never stops a
    /// carry: but for `Stripped`, each goes on as it did where it was,
    /// whatever the section it lands in says.
    ///
    /// [`move_group`]: Plan::move_group
    pub fn carry(
        &mut self,
        steps: &[StepId],
        to: Place,
        how: Carry,
    ) -> Result<Carried, CarryError> {
        if steps.is_empty() {
            return Err(CarryError::Missing);
        }
        for id in steps {
            let step = self.get(*id).ok_or(CarryError::Missing)?;
            if step.is_section() {
                return Err(CarryError::Heading(*id));
            }
        }
        let carried: Vec<StepId> = self
            .steps
            .iter()
            .map(|step| step.id)
            .filter(|id| steps.contains(id))
            .collect();
        let (mut next, rows) = self.without(&carried);
        next.lay_in(rows, to)?;
        // Each span's carried steps, and whether it is crossed.
        let mut crossed = Vec::new();
        let mut among = Vec::new();
        for span in &self.spans {
            let held = self.span_steps(span.id);
            let moving: Vec<StepId> = held
                .iter()
                .copied()
                .filter(|id| carried.contains(id))
                .collect();
            if moving.is_empty() {
                continue;
            }
            let ranged = matches!(span.over, Over::Steps { .. });
            if ranged && moving.len() == held.len() {
                among.push(span.id);
                continue;
            }
            let after = next.span_steps(span.id);
            if !moving.iter().all(|id| after.contains(id)) {
                crossed.push(span.id);
            }
        }
        if how == Carry::Whole && !crossed.is_empty() {
            return Err(CarryError::Crosses(crossed));
        }
        for id in &crossed {
            next.too_few(*id);
        }
        for id in &carried {
            let Some(at) = next.position(*id) else {
                continue;
            };
            let advance = match how {
                Carry::Stripped => Some(false),
                Carry::Whole | Carry::CutLoose => {
                    let was = self.position(*id).map(|was| self.goes_on(was));
                    was.filter(|was| *was != next.goes_on(at))
                }
            };
            if advance.is_some() {
                next.steps[at].advance = advance;
            }
        }
        if how == Carry::Stripped {
            for id in &among {
                next.remove_span(*id);
            }
        }
        next.settle();
        let gone = self
            .spans
            .iter()
            .map(|span| span.id)
            .filter(|id| next.span(*id).is_none())
            .collect();
        *self = next;
        Ok(Carried {
            steps: carried,
            left: crossed,
            gone,
        })
    }

    /// A span from step to step left with too few steps for what it
    /// does: no loop round one step, and no drawn order of two. One
    /// attached to a section keeps what it does.
    fn too_few(&mut self, id: SpanId) {
        let count = self.span_steps(id).len();
        let Some(span) = self.spans.iter_mut().find(|span| span.id == id) else {
            return;
        };
        if !matches!(span.over, Over::Steps { .. }) {
            return;
        }
        if count < 3 {
            span.flow.shuffle = false;
        }
        if count < 2 {
            span.flow.repeat = Repeat::Once;
            span.flow.ends = Ending::default();
        }
    }

    /// A new plan titled `title` from this one as a template: its sections,
    /// steps, notes, live slots and their searches, lengths and look, with
    /// nothing a service left in it (what the live slots showed). Not a
    /// template itself.
    pub fn from_template(&self, title: impl Into<String>) -> Plan {
        let mut plan = self.clone();
        plan.title = title.into();
        plan.template = false;
        for step in &mut plan.steps {
            if let Action::Live { shown, .. } = &mut step.action {
                shown.clear();
            }
        }
        plan
    }

    /// Serializes to pretty JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("plans always serialize")
    }

    /// Parses a plan from JSON.
    pub fn from_json(json: &str) -> Result<Self, PlanError> {
        let saved: serde_json::Value = serde_json::from_str(json)?;
        let ended = ends_before_spans(&saved);
        let mut plan: Plan = serde_json::from_value(saved)?;
        // Hand-edited files may have a stale counter; never reuse an id.
        let max = plan.steps.iter().map(|s| s.id.0 + 1).max().unwrap_or(0);
        plan.next_id = plan.next_id.max(max);
        let max = plan.spans.iter().map(|s| s.id.0 + 1).max().unwrap_or(0);
        plan.next_span = plan.next_span.max(max);
        plan.upgrade_before_spans(&ended);
        plan.upgrade_step_notes();
        plan.settle();
        Ok(plan)
    }

    /// Writes the plan to a file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), PlanError> {
        std::fs::write(path, self.to_json())?;
        Ok(())
    }

    /// Reads a plan from a file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PlanError> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }
}

/// In a plan saved before there were spans, a section could end at a
/// step: `"through"` in its action. Each such section's id, with the id
/// of that step. None in a plan saved since.
fn ends_before_spans(saved: &serde_json::Value) -> Vec<(StepId, StepId)> {
    let steps = saved.get("steps").and_then(|steps| steps.as_array());
    steps
        .into_iter()
        .flatten()
        .filter_map(|step| {
            let id = step.get("id")?.as_u64()?;
            let last = step.get("action")?.get("through")?.as_u64()?;
            Some((StepId(id), StepId(last)))
        })
        .collect()
}

/// Lengths as whole seconds in JSON, which people can read and edit.
mod seconds {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(duration) => s.serialize_some(&duration.as_secs()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        Ok(Option::<u64>::deserialize(d)?.map(Duration::from_secs))
    }
}

#[cfg(test)]
mod tests {
    use lantern_resource::{Kind, Location, ResourceId, SourceId};

    use super::*;

    fn resource(title: &str) -> Resource {
        let source = SourceId::new("test").unwrap();
        Resource::new(
            ResourceId::new(&source, title),
            title,
            Kind::Image,
            Location::Url(title.into()),
        )
    }

    fn labels(plan: &Plan) -> Vec<&str> {
        plan.steps().iter().map(|s| s.label.as_str()).collect()
    }

    #[test]
    fn editing_keeps_ids_stable() {
        let mut plan = Plan::new("Sunday");
        let welcome = plan.show(resource("Welcome"));
        let worship = plan.live("Worship", None);
        let prayer = plan.push("Prayer", Action::Blank);
        assert_eq!(labels(&plan), ["Welcome", "Worship", "Prayer"]);

        assert!(plan.move_to(prayer, 0));
        assert_eq!(labels(&plan), ["Prayer", "Welcome", "Worship"]);
        assert_eq!(plan.position(welcome), Some(1));

        plan.remove(worship);
        let notes = plan.insert(99, "Notes", Action::Note);
        assert_eq!(labels(&plan), ["Prayer", "Welcome", "Notes"]);
        assert_ne!(notes, worship);
    }

    #[test]
    fn a_section_carries_its_steps_and_moves_as_one() {
        let mut plan = Plan::new("Sunday");
        let welcome = plan.show(resource("Welcome"));
        let worship = plan.section("Worship", None);
        let song = plan.show(resource("Song"));
        let hymn = plan.show(resource("Hymn"));
        let word = plan.section("Word", None);
        let reading = plan.show(resource("Reading"));
        assert_eq!(plan.carried(worship), [worship, song, hymn]);
        assert_eq!(plan.carried(word), [word, reading]);
        assert_eq!(plan.carried(song), [song]);

        // After Word's steps: index 3 in [Welcome, Word, Reading].
        assert!(plan.move_steps(&plan.carried(worship), 3));
        assert_eq!(
            labels(&plan),
            ["Welcome", "Word", "Reading", "Worship", "Song", "Hymn"]
        );
        assert_eq!(plan.position(welcome), Some(0));
        assert!(!plan.move_steps(&[StepId(999)], 0));
    }

    #[test]
    fn json_round_trip_and_id_counter_repair() {
        let mut plan = Plan::new("Sunday");
        plan.show(resource("Welcome"));
        plan.live("Worship", Some(Query::new("").with_kind(Kind::Video)));
        let back = Plan::from_json(&plan.to_json()).unwrap();
        assert_eq!(plan, back);

        let stale = plan.to_json().replace("\"next_id\": 2", "\"next_id\": 0");
        let mut repaired = Plan::from_json(&stale).unwrap();
        let id = repaired.push("New", Action::Note);
        assert_eq!(id, StepId(2));
    }

    #[test]
    fn a_plan_keeps_its_own_look_and_old_files_use_the_default() {
        let mut plan = Plan::new("Santa Cena");
        assert!(!plan.to_json().contains("look"), "no look, nothing written");
        plan.look = Some(Look {
            background: lantern_showman::Rgb([0x30, 0x10, 0x10]),
            ..Look::default()
        });
        let back = Plan::from_json(&plan.to_json()).unwrap();
        assert_eq!(back.look, plan.look);
        let old = Plan::from_json(r#"{"title": "Old", "steps": []}"#).unwrap();
        assert_eq!(old.look, None);
    }

    #[test]
    fn a_template_starts_a_plan_with_its_order_but_not_its_service() {
        let mut template = Plan::new("Domingo");
        template.template = true;
        template.section("Alabanza", Some(Duration::from_secs(15 * 60)));
        let slot = template.live("Himnos", Some(Query::new("!h")));
        let welcome = template.show(resource("Bienvenida"));
        template.get_mut(welcome).unwrap().advance = Some(true);
        if let Action::Live { shown, .. } = &mut template.get_mut(slot).unwrap().action {
            shown.push(resource("Himno 436"));
        }
        let json = template.to_json();
        assert!(json.contains("\"template\": true"), "{json}");

        let sunday = template.from_template("Domingo 2026-11-15");
        assert_eq!(sunday.title, "Domingo 2026-11-15");
        assert!(!sunday.template);
        assert_eq!(labels(&sunday), labels(&template));
        assert_eq!(
            sunday.get(welcome).unwrap().advance,
            Some(true),
            "settings come along"
        );
        match &sunday.get(slot).unwrap().action {
            Action::Live { hint, shown } => {
                assert_eq!(hint.as_ref().map(|q| q.text.as_str()), Some("!h"));
                assert!(shown.is_empty(), "last service's shows stay behind");
            }
            other => panic!("{other:?}"),
        }
        assert!(!Plan::new("Plain").to_json().contains("template"));
        // Its own folders are saved with it, and go with a plan from it.
        assert!(!Plan::new("Plain").to_json().contains("folders"));
        template.folders = vec!["~/Church/Campaña".into()];
        let back = Plan::from_json(&template.to_json()).unwrap();
        assert_eq!(back.folders, template.folders);
        assert_eq!(template.from_template("Next").folders, template.folders);
        // And when it starts: every Sunday at the same time.
        assert!(!Plan::new("Plain").to_json().contains("starts"));
        template.starts = TimeOfDay::parse("10:30");
        assert!(template.to_json().contains("\"starts\": \"10:30\""));
        let back = Plan::from_json(&template.to_json()).unwrap();
        assert_eq!(back.starts, template.starts);
        assert_eq!(template.from_template("Next").starts, template.starts);
        // And what it keeps on standby, once.
        assert!(!Plan::new("Plain").to_json().contains("standby"));
        assert!(template.keep_ready(resource("Move your car")));
        assert!(!template.keep_ready(resource("Move your car")));
        let back = Plan::from_json(&template.to_json()).unwrap();
        assert_eq!(back.standby, template.standby);
        assert_eq!(template.from_template("Next").standby.len(), 1);
        // So are the Bibles it searches.
        assert!(!Plan::new("Plain").to_json().contains("bibles"));
        template.bibles = vec!["RV1909".into()];
        let back = Plan::from_json(&template.to_json()).unwrap();
        assert_eq!(back.bibles, ["RV1909"]);
        assert_eq!(template.from_template("Next").bibles, ["RV1909"]);
    }

    #[test]
    fn sections_and_estimates_save_as_seconds() {
        let mut plan = Plan::new("Sunday");
        plan.section("Alabanza", Some(Duration::from_secs(15 * 60)));
        let song = plan.show(resource("Sublime gracia"));
        plan.get_mut(song).unwrap().estimate = Some(Duration::from_secs(240));
        let json = plan.to_json();
        assert!(json.contains("\"budget\": 900"), "{json}");
        assert!(json.contains("\"estimate\": 240"), "{json}");
        assert_eq!(Plan::from_json(&json).unwrap(), plan);
        assert!(plan.steps()[0].is_section() && !plan.steps()[1].is_section());
    }

    #[test]
    fn save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.json");
        let mut plan = Plan::new("Sunday");
        plan.push("Prayer", Action::Blank);
        plan.save(&path).unwrap();
        assert_eq!(Plan::load(&path).unwrap(), plan);
    }

    /// Music { Praise { A, B }, Reading { C } }, Sermon { D }.
    fn nested() -> (Plan, [StepId; 4]) {
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        let praise = plan.section("Praise", None);
        plan.show(resource("A"));
        plan.show(resource("B"));
        let reading = plan.section("Reading", None);
        plan.show(resource("C"));
        let sermon = plan.section("Sermon", None);
        plan.show(resource("D"));
        assert!(plan.nest(praise, true));
        assert!(plan.nest(reading, true));
        (plan, [music, praise, reading, sermon])
    }

    #[test]
    fn a_section_holds_the_sections_in_it() {
        let (plan, [music, praise, reading, sermon]) = nested();
        let depths: Vec<_> = plan.steps().iter().filter_map(Step::depth).collect();
        assert_eq!(depths, [0, 1, 1, 0]);
        // Music holds everything up to Sermon; Praise only its two.
        assert_eq!(plan.carried(music).len(), 6);
        assert_eq!(plan.carried(praise).len(), 3);
        assert_eq!(plan.carried(reading).len(), 2);
        assert_eq!(plan.carried(sermon).len(), 2);
        // Rows are drawn one further in than the section they are in.
        let levels: Vec<_> = (0..plan.len()).map(|at| plan.level(at)).collect();
        assert_eq!(levels, [0, 1, 2, 2, 1, 2, 0, 1]);
        // C is in Music, then Reading.
        assert_eq!(plan.within(5), [music, reading]);
        assert_eq!(plan.within(1), [music]);
        assert!(plan.within(0).is_empty());
    }

    #[test]
    fn a_section_moves_with_its_level_and_what_it_holds() {
        let mut plan = Plan::new("Sabbath");
        let singing = plan.section("Singing", None);
        let special = plan.section("Special", None);
        let duet = plan.section("Duet", None);
        plan.nest(duet, true);
        let school = plan.section("School", None);
        let depths =
            |plan: &Plan| -> Vec<u8> { plan.steps().iter().filter_map(Step::depth).collect() };
        assert_eq!(depths(&plan), [0, 0, 1, 0]);
        // Special, with Duet in it, stays where it is and goes into
        // Singing: Duet goes in as far.
        let carried = plan.carried(special);
        assert_eq!(carried, [special, duet]);
        assert!(plan.move_group(&carried, 1, 1));
        assert_eq!(depths(&plan), [0, 1, 2, 0]);
        assert_eq!(
            plan.within(plan.position(duet).unwrap()),
            [singing, special]
        );
        // To the end, out at the plan's own level again.
        assert!(plan.move_group(&carried, 2, 0));
        assert_eq!(depths(&plan), [0, 0, 0, 1]);
        assert_eq!(plan.position(school), Some(1));
        // Nothing goes further in than there is a section to hold it.
        assert!(plan.move_group(&carried, 0, 2));
        assert_eq!(depths(&plan), [0, 1, 0, 0], "first is the plan's own");
    }

    /// Music: A B C D; Sermon: E.
    fn hymns() -> (Plan, StepId, [StepId; 5]) {
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        let steps = ["A", "B", "C", "D"].map(|name| plan.show(resource(name)));
        plan.section("Sermon", None);
        let e = plan.show(resource("E"));
        (plan, music, [steps[0], steps[1], steps[2], steps[3], e])
    }

    /// The steps a span is over, by their labels.
    fn over(plan: &Plan, span: SpanId) -> Vec<&str> {
        let (first, last) = plan.span_range(span).expect("over some steps");
        plan.steps()[first..=last]
            .iter()
            .filter(|step| !step.is_section())
            .map(|step| step.label.as_str())
            .collect()
    }

    /// The spans a step is in, outermost first.
    fn spans_of(plan: &Plan, id: StepId) -> Vec<SpanId> {
        plan.spans_at(plan.position(id).unwrap())
    }

    #[test]
    fn a_loop_is_a_span_over_some_steps_and_no_row_of_the_plan() {
        let (mut plan, music, [a, b, c, d, _]) = hymns();
        // From C back up to A, as it is dragged.
        let looped = plan.loop_over(c, a).expect("a loop");
        assert_eq!(plan.len(), 7, "nothing was put in");
        let span = *plan.span(looped).unwrap();
        assert_eq!(span.over, Over::Steps { first: a, last: c });
        assert_eq!(span.flow.repeat, Repeat::Times(2));
        assert_eq!(plan.span_range(looped), Some((1, 3)));
        assert_eq!(over(&plan, looped), ["A", "B", "C"]);
        // What it is over is in it; D after it is not. Both are in Music
        // and at Music's level, as before.
        assert_eq!(spans_of(&plan, b), [looped]);
        assert!(spans_of(&plan, d).is_empty());
        let within = |plan: &Plan, id| plan.within(plan.position(id).unwrap());
        assert_eq!(within(&plan, b), [music]);
        assert_eq!((plan.level(2), plan.level(4)), (1, 1));
        // It reads back the same.
        let json = plan.to_json();
        assert!(json.contains("\"spans\""), "{json}");
        assert_eq!(Plan::from_json(&json).unwrap(), plan);
        // How it moves can be changed, and it can be taken away.
        let forever = Flow {
            repeat: Repeat::Forever,
            ..Flow::default()
        };
        assert_eq!(plan.set_flow(looped, forever), Ok(()));
        assert_eq!(plan.flow_of(looped), Some(forever));
        assert!(plan.unloop(looped));
        assert!(plan.spans().is_empty());
        assert!(!plan.to_json().contains("\"spans\""));
        assert_eq!(plan.len(), 7, "the steps stay");
    }

    #[test]
    fn a_loop_goes_across_a_heading_and_round_two_steps_or_more() {
        let (mut plan, _, [a, _, _, d, e]) = hymns();
        // Not round one step.
        assert_eq!(plan.loop_over(a, a), Err(SpanError::TooFew));
        assert_eq!(plan.loop_over(a, StepId(999)), Err(SpanError::Missing));
        // From D in Music to E in Sermon: the heading is no bar.
        let looped = plan.loop_over(e, d).expect("across the heading");
        assert_eq!(plan.span_range(looped), Some((4, 6)));
        assert_eq!(over(&plan, looped), ["D", "E"]);
        // The heading between them is among its rows.
        assert_eq!(plan.spans_at(5), [looped]);
        // Going on from D is E, over Sermon's heading.
        let at = |id| plan.position(id).unwrap();
        assert_eq!(plan.next_step(at(d)), Some(at(e)));
        assert_eq!(plan.next_step(at(e)), None);
        assert_eq!(plan.would_shuffle_over(d, e), Err(SpanError::TooFew));
        assert_eq!(plan.would_loop_section(d), Err(SpanError::Missing));
        // Drawn from a heading, it starts at the first step after it.
        let (mut plan, music, [a, b, ..]) = hymns();
        let looped = plan.loop_over(music, b).unwrap();
        assert_eq!(
            plan.span(looped).unwrap().over,
            Over::Steps { first: a, last: b }
        );
    }

    #[test]
    fn a_span_attached_to_a_section_takes_in_what_is_put_there_later() {
        let (mut plan, music, [a, _, _, d, e]) = hymns();
        let looped = plan.loop_section(music).unwrap();
        assert_eq!(plan.span(looped).unwrap().over, Over::Section(music));
        assert_eq!(plan.section_span(music).map(|span| span.id), Some(looped));
        assert_eq!(over(&plan, looped), ["A", "B", "C", "D"]);
        // The heading itself is not in it: it is over the steps.
        assert!(plan.spans_at(0).is_empty());
        // A new step in Music is in the loop; one in Sermon is not.
        let new = plan.insert(5, "New", Action::Blank);
        assert_eq!(spans_of(&plan, new), [looped]);
        assert_eq!(over(&plan, looped), ["A", "B", "C", "D", "New"]);
        assert!(spans_of(&plan, e).is_empty());
        // So is one moved in, and one moved out is out.
        plan.move_steps(&[e], 1);
        plan.move_steps(&[a], 99);
        assert_eq!(over(&plan, looped), ["E", "B", "C", "D", "New"]);
        // A section's heading says how its span moves.
        assert_eq!(
            plan.flow_of(music).map(|flow| flow.repeat),
            Some(Repeat::Times(2))
        );
        // A section with no steps yet can be looped, for what will come.
        let empty = plan.section("Later", None);
        let later = plan.loop_section(empty).unwrap();
        assert_eq!(plan.span_range(later), None);
        let step = plan.push("Step", Action::Blank);
        assert_eq!(spans_of(&plan, step), [later]);
        // Only a section has one.
        assert_eq!(plan.loop_section(step), Err(SpanError::Missing));
        // Without its section, it is gone; the steps stay.
        plan.remove(music);
        assert_eq!(plan.span(looped), None);
        assert!(plan.position(d).is_some());
    }

    #[test]
    fn a_range_that_has_a_span_already_changes_that_span() {
        let (mut plan, music, [a, b, _, d, _]) = hymns();
        // B to D, in a drawn order.
        let drawn = plan.shuffle_over(d, b).expect("a drawn order");
        assert!(plan.span(drawn).unwrap().flow.shuffle);
        // The same three again: the span they have comes again too.
        assert_eq!(plan.loop_over(b, d), Ok(drawn));
        assert_eq!(plan.span(drawn).unwrap().flow.repeat, Repeat::Times(2));
        assert_eq!(plan.spans().len(), 1);
        // Neither taken away alone takes the span; both do.
        assert!(plan.unshuffle(drawn));
        assert!(plan.span(drawn).is_some());
        assert!(plan.unloop(drawn));
        assert_eq!(plan.span(drawn), None);
        // Round all Music holds, from step to step, then Music itself:
        // one span, now attached to the section.
        let looped = plan.loop_over(a, d).unwrap();
        assert_eq!(plan.loop_section(music), Ok(looped));
        assert_eq!(plan.span(looped).unwrap().over, Over::Section(music));
        assert_eq!(plan.spans().len(), 1);
        // And from step to step again, it is still the section's.
        assert_eq!(plan.loop_over(a, d), Ok(looped));
        assert_eq!(plan.span(looped).unwrap().over, Over::Section(music));
    }

    #[test]
    fn a_drawn_order_needs_three_steps_and_stays_in_one_section() {
        let (mut plan, music, [a, b, c, d, e]) = hymns();
        // Two steps have no order worth drawing.
        assert_eq!(plan.shuffle_over(a, b), Err(SpanError::TooFew));
        // Not across Sermon's heading, though a loop goes there.
        assert_eq!(plan.shuffle_over(c, e), Err(SpanError::AcrossHeading));
        let looped = plan.loop_over(c, e).unwrap();
        let drawn = Flow {
            shuffle: true,
            ..plan.span(looped).unwrap().flow
        };
        assert_eq!(plan.set_flow(looped, drawn), Err(SpanError::AcrossHeading));
        assert!(!plan.span(looped).unwrap().flow.shuffle, "nothing changed");
        plan.remove_span(looped);
        // A loop round two steps can't be given a drawn order either.
        let pair = plan.loop_over(a, b).unwrap();
        assert_eq!(plan.set_flow(pair, drawn), Err(SpanError::TooFew));
        plan.remove_span(pair);
        // All at the plan's own level is one place too.
        let mut flat = Plan::new("Flat");
        let [x, _, z] = ["X", "Y", "Z"].map(|name| flat.push(name, Action::Blank));
        assert!(flat.shuffle_over(x, z).is_ok());
        // A section's own, while no heading is among its steps.
        let drawn = plan.shuffle_section(music).unwrap();
        assert!(plan.span(drawn).unwrap().flow.shuffle);
        // A heading that comes among them ends it: the plan's order.
        let inner = plan.insert(
            3,
            "Inner",
            Action::Section {
                budget: None,
                depth: 0,
                last: None,
                done_by: None,
            },
        );
        assert!(plan.nest(inner, true));
        assert_eq!(plan.span(drawn), None);
        assert_eq!(
            plan.shuffle_section(music),
            Err(SpanError::AcrossHeading),
            "Music holds a heading with steps before it"
        );
        let _ = d;
    }

    #[test]
    fn two_spans_are_apart_or_one_is_wholly_in_the_other() {
        let mut plan = Plan::new("Sabbath");
        let ids = ["A", "B", "C", "D", "E"].map(|name| plan.show(resource(name)));
        let [a, b, c, d, e] = ids;
        let drawn = plan.shuffle_over(b, d).unwrap();
        // From A to C would cut the drawn order in two; so would C to E.
        assert_eq!(plan.loop_over(a, c), Err(SpanError::HalfOver(drawn)));
        assert_eq!(plan.loop_over(c, e), Err(SpanError::HalfOver(drawn)));
        assert_eq!(plan.spans().len(), 1, "nothing was laid");
        // From A to D the drawn order is all among them: round it.
        let outer = plan.loop_over(a, d).unwrap();
        assert_ne!(outer, drawn);
        assert_eq!(spans_of(&plan, b), [outer, drawn]);
        assert_eq!(spans_of(&plan, a), [outer]);
        assert!(spans_of(&plan, e).is_empty());
        // And inside it, round two of its steps.
        let inner = plan.loop_over(b, c).unwrap();
        assert_eq!(spans_of(&plan, c), [outer, drawn, inner]);
        assert_eq!(
            plan.span_ranges(),
            [(outer, 0, 3), (drawn, 1, 3), (inner, 1, 2)]
        );
        let laid: Vec<_> = plan
            .laid_spans()
            .into_iter()
            .map(|(span, first, last)| (span.id, span.is_loop(), span.is_drawn(), first, last))
            .collect();
        assert_eq!(
            laid,
            [
                (outer, true, false, 0, 3),
                (drawn, false, true, 1, 3),
                (inner, true, false, 1, 2)
            ]
        );
        // A dry run says the same as the real thing and changes nothing.
        let before = plan.clone();
        assert_eq!(plan.would_loop_over(c, e), Err(SpanError::HalfOver(drawn)));
        assert!(plan.would_loop_over(d, e).is_err());
        assert_eq!(plan.would_shuffle_over(a, b), Err(SpanError::TooFew));
        assert_eq!(plan, before);
        assert_eq!(Plan::from_json(&plan.to_json()).unwrap(), plan);
        // A section's span keeps to it as well: one from in it to out
        // of it would cut it.
        let (mut plan, music, [_, _, c, d, e]) = hymns();
        let musics = plan.loop_section(music).unwrap();
        assert_eq!(plan.loop_over(d, e), Err(SpanError::HalfOver(musics)));
        assert!(plan.loop_over(c, d).is_ok());
        // Edits that leave two half over each other take the newer away.
        let (mut plan, _, [_, _, c, d, _]) = hymns();
        let sermon = plan.steps()[5].id;
        let first = plan.loop_section(sermon).unwrap();
        let second = plan.loop_over(c, d).unwrap();
        // Sermon's heading moves up between C and D: D is in Sermon now.
        plan.move_steps(&[sermon], 4);
        assert_eq!(over(&plan, first), ["D", "E"]);
        assert_eq!(plan.span(second), None);
    }

    #[test]
    fn a_span_follows_its_steps_as_they_come_and_go() {
        let (mut plan, _, [a, b, c, d, _]) = hymns();
        let looped = plan.loop_over(a, c).unwrap();
        // Its last step moved out: it ends at the one before.
        plan.move_steps(&[c], 99);
        assert_eq!(over(&plan, looped), ["A", "B"]);
        // Its last step removed: the same.
        plan.remove(b);
        assert_eq!(over(&plan, looped), ["A"]);
        // With nothing left in it, it is gone.
        plan.remove(a);
        assert_eq!(plan.span(looped), None);
        // A step put between its steps is in it.
        let (mut plan, _, [a, b, c, ..]) = hymns();
        let looped = plan.loop_over(a, c).unwrap();
        let at = plan.position(b).unwrap();
        plan.move_steps(&[d], at);
        assert_eq!(over(&plan, looped), ["A", "D", "B", "C"]);
        // Its first step moved out: it starts at the next.
        plan.move_steps(&[a], 99);
        assert_eq!(over(&plan, looped), ["D", "B", "C"]);
        // Its last step moved in among the others stays in it.
        plan.move_to(c, 2);
        assert_eq!(over(&plan, looped), ["D", "C", "B"]);
        // A step put back where it is changes nothing, an end of it too.
        let before = plan.clone();
        plan.move_to(c, 2);
        plan.move_to(d, 1);
        assert_eq!(plan, before);
        // All its steps moved together: it goes with them.
        let end = plan.len() - 3;
        plan.move_steps(&[c, d, b], end);
        assert_eq!(over(&plan, looped), ["D", "C", "B"]);
        assert_eq!(plan.span_range(looped), Some((4, 6)));
        // A section moved with its steps takes the spans in it along.
        let (mut plan, music, [a, b, ..]) = hymns();
        let looped = plan.loop_over(a, b).unwrap();
        let carried = plan.carried(music);
        assert!(plan.move_group(&carried, 2, 0));
        assert_eq!(labels(&plan)[..3], ["Sermon", "E", "Music"]);
        assert_eq!(over(&plan, looped), ["A", "B"]);
    }

    #[test]
    fn a_plan_saved_before_spans_loads_with_its_loops_as_spans() {
        // Saved by the plan as it was: Welcome twice; Music three times
        // round, holding a loop with no name round A to D (until 10:30,
        // there and then) and in it a drawn order round B to D, then E;
        // Sermon in a drawn order, holding F and G; all of it forever.
        let plan = Plan::from_json(include_str!("../fixtures/plan_before_spans.json")).unwrap();
        // The loops with no name are no rows now.
        assert_eq!(
            labels(&plan),
            [
                "Welcome", "Music", "A", "B", "C", "D", "E", "Sermon", "F", "G"
            ]
        );
        let id = |label: &str| plan.steps().iter().find(|s| s.label == label).unwrap().id;
        let flow_over = |over: Over| {
            let span = plan.spans().iter().find(|span| span.over == over);
            span.expect("a span").flow
        };
        assert_eq!(plan.spans().len(), 4);
        // Music's and Sermon's are attached to them.
        assert_eq!(
            flow_over(Over::Section(id("Music"))),
            Flow {
                repeat: Repeat::Times(3),
                ..Flow::default()
            }
        );
        assert_eq!(
            flow_over(Over::Section(id("Sermon"))),
            Flow {
                shuffle: true,
                ..Flow::default()
            }
        );
        // The two with no name are from step to step.
        let (first, last) = (id("A"), id("D"));
        assert_eq!(
            flow_over(Over::Steps { first, last }),
            Flow {
                repeat: Repeat::Until(TimeOfDay::parse("10:30").unwrap()),
                ends: Ending::Sharp,
                ..Flow::default()
            }
        );
        let (first, last) = (id("B"), id("D"));
        assert_eq!(
            flow_over(Over::Steps { first, last }),
            Flow {
                shuffle: true,
                ..Flow::default()
            }
        );
        // No heading says how it moves by itself, a step still does, and
        // the plan keeps its own.
        assert!(
            plan.steps()
                .iter()
                .all(|s| !s.is_section() || s.flow.is_usual())
        );
        assert_eq!(
            plan.get(id("Welcome")).unwrap().flow.repeat,
            Repeat::Times(2)
        );
        assert_eq!(plan.flow.repeat, Repeat::Forever);
        // Nested as they were: Music's, the loop, the drawn order.
        let nested: Vec<Over> = plan
            .spans_at(plan.position(id("C")).unwrap())
            .into_iter()
            .map(|span| plan.span(span).unwrap().over)
            .collect();
        assert_eq!(
            nested,
            [
                Over::Section(id("Music")),
                Over::Steps {
                    first: id("A"),
                    last: id("D")
                },
                Over::Steps {
                    first: id("B"),
                    last: id("D")
                },
            ]
        );
        // Saved again it is in today's form, and reads back the same.
        let json = plan.to_json();
        assert!(!json.contains("through"), "{json}");
        assert_eq!(Plan::from_json(&json).unwrap(), plan);
        // New ids don't reuse the ones the loops had.
        let mut plan = plan;
        assert_eq!(plan.push("New", Action::Note), StepId(12));
    }

    #[test]
    fn a_step_put_there_by_a_fill_says_so_and_is_kept_with_the_plan() {
        let mut plan = Plan::new("Sabbath");
        let picked = plan.show(resource("Picked"));
        let filled = plan.show(resource("Filled"));
        plan.get_mut(filled).unwrap().filled = true;
        let json = plan.to_json();
        assert_eq!(json.matches("\"filled\"").count(), 1, "only where it is so");
        let back = Plan::from_json(&json).unwrap();
        assert!(!back.get(picked).unwrap().filled);
        assert!(back.get(filled).unwrap().filled);
    }

    #[test]
    fn nesting_has_its_limits() {
        let (mut plan, [music, praise, reading, _]) = nested();
        // The first section has nothing to be a part of.
        assert!(!plan.nest(music, true));
        // Praise is the first in Music: nothing before it at its level.
        assert!(!plan.nest(praise, true));
        // Reading can go into Praise, and no further than the deepest.
        assert!(plan.nest(reading, true));
        assert_eq!(plan.get(reading).and_then(Step::depth), Some(DEEPEST));
        assert!(!plan.nest(reading, true));
        // Out again, twice, and no further than the plan's own level.
        assert!(plan.nest(reading, false));
        assert!(plan.nest(reading, false));
        assert!(!plan.nest(reading, false));
    }

    #[test]
    fn a_section_moves_with_what_it_holds_and_none_is_left_stranded() {
        let (mut plan, [music, praise, _, sermon]) = nested();
        // Moving Music takes its sections along.
        let ids = plan.carried(music);
        assert!(plan.move_steps(&ids, 2));
        assert_eq!(
            labels(&plan),
            ["Sermon", "D", "Music", "Praise", "A", "B", "Reading", "C"]
        );
        // Removing Music leaves Praise with nothing to be in: it moves out.
        plan.remove(music);
        assert_eq!(plan.get(praise).and_then(Step::depth), Some(0));
        assert_eq!(plan.get(sermon).and_then(Step::depth), Some(0));
    }

    #[test]
    fn depth_is_saved_only_when_there_is_some() {
        let (plan, [_, praise, ..]) = nested();
        let json = plan.to_json();
        assert_eq!(json.matches("\"depth\"").count(), 2);
        let back = Plan::from_json(&json).unwrap();
        assert_eq!(back.get(praise).and_then(Step::depth), Some(1));
    }

    #[test]
    fn the_look_carries_down_and_each_level_can_change_it() {
        use lantern_showman::Rgb;
        let (mut plan, [music, _, reading, _]) = nested();
        let usual = Look::default();
        let red = Rgb([0x80, 0, 0]);
        let blue = Rgb([0, 0, 0x80]);
        // Music turns the background red for everything in it.
        plan.get_mut(music).unwrap().look = Some(LookChange {
            background: Some(red),
            ..LookChange::default()
        });
        // Reading, inside it, sets a font and keeps the red.
        plan.get_mut(reading).unwrap().look = Some(LookChange {
            font: Some("Georgia".into()),
            ..LookChange::default()
        });
        // C, in Reading, has its own background.
        let c = plan.steps()[5].id;
        plan.get_mut(c).unwrap().look = Some(LookChange {
            background: Some(blue),
            ..LookChange::default()
        });
        let at = |label: &str| plan.steps().iter().position(|s| s.label == label).unwrap();
        assert_eq!(plan.look_at(at("A"), &usual).background, red);
        assert_eq!(plan.look_at(at("A"), &usual).font, None);
        assert_eq!(plan.look_at(at("C"), &usual).background, blue);
        assert_eq!(
            plan.look_at(at("C"), &usual).font.as_deref(),
            Some("Georgia")
        );
        // Sermon is outside Music: the look for all plans.
        assert_eq!(plan.look_at(at("D"), &usual), usual);
        // A plan with its own look starts from that.
        let c_at = at("C");
        plan.look = Some(Look {
            text: red,
            ..Look::default()
        });
        assert_eq!(plan.look_at(c_at, &usual).text, red);
        assert_eq!(plan.look_at(c_at, &usual).background, blue);
    }

    #[test]
    fn a_section_is_a_plan_of_its_own() {
        let (mut plan, [music, praise, _, sermon]) = nested();
        plan.get_mut(music).unwrap().advance = Some(true);
        plan.loop_section(music).unwrap();
        let (a, b) = (plan.steps()[2].id, plan.steps()[3].id);
        let inner = plan.loop_over(a, b).unwrap();
        let part = plan.part(music).unwrap();
        assert_eq!(labels(&part), ["Praise", "A", "B", "Reading", "C"]);
        // The sections in it are its own sections now.
        assert_eq!(part.get(praise).and_then(Step::depth), Some(0));
        // It moves as Music did.
        assert!(part.advance);
        assert_eq!(part.flow.repeat, Repeat::Times(2));
        assert!(part.goes_on(1));
        // The loop in it came along; Music's own is the part's flow.
        assert_eq!(part.spans().len(), 1);
        assert_eq!(part.span_range(inner), Some((1, 2)));
        // A span is a plan of its own too: once round it.
        let round = plan.span_part(inner).unwrap();
        assert_eq!(labels(&round), ["A", "B"]);
        assert!(round.spans().is_empty());
        assert_eq!(round.flow.repeat, Repeat::Times(2));
        // The same steps, by the same ids.
        assert_eq!(part.steps()[1].id, plan.steps()[2].id);
        assert_eq!(labels(&plan.part(sermon).unwrap()), ["D"]);
        assert!(plan.part(part.steps()[1].id).is_none(), "a step isn't one");
    }

    #[test]
    fn an_order_drawn_by_hand_grows_what_it_meets_and_rubs_out_what_it_is_inside() {
        let mut plan = Plan::new("Sabbath");
        let steps: Vec<StepId> = ["A", "B", "C", "D", "E", "F", "G", "H"]
            .into_iter()
            .map(|name| plan.push(name, Action::Blank))
            .collect();
        let at = |n: usize| steps[n];
        let orders = |plan: &Plan| -> Vec<(usize, usize)> {
            plan.laid_spans()
                .into_iter()
                .filter(|(span, ..)| span.flow.shuffle)
                .map(|(_, first, last)| (first, last))
                .collect()
        };
        // Over nothing drawn: an order of its own.
        assert!(matches!(plan.draw_over(at(0), at(2)), Ok(Drawn::Made(_))));
        assert_eq!(orders(&plan), vec![(0, 2)]);
        // One that only touches it stays apart: each is drawn by itself.
        assert!(matches!(plan.draw_over(at(3), at(5)), Ok(Drawn::Made(_))));
        assert_eq!(orders(&plan), vec![(0, 2), (3, 5)]);
        // One that starts on the last step of another joins it; and here
        // it meets the second too, so all three are one.
        assert!(matches!(plan.draw_over(at(2), at(3)), Ok(Drawn::Grown(_))));
        assert_eq!(orders(&plan), vec![(0, 5)]);
        // Growing on past its end.
        assert!(matches!(plan.draw_over(at(5), at(7)), Ok(Drawn::Grown(_))));
        assert_eq!(orders(&plan), vec![(0, 7)]);
        // Inside it: that part is rubbed out, and each side stays an
        // order where three are left.
        assert_eq!(plan.would_draw_over(at(3), at(4)), Ok(Drawn::Rubbed));
        assert_eq!(orders(&plan), vec![(0, 7)], "a dry run changes nothing");
        assert_eq!(plan.draw_over(at(3), at(4)), Ok(Drawn::Rubbed));
        assert_eq!(orders(&plan), vec![(0, 2), (5, 7)]);
        // Two left is no order: rubbing one step off the end of three
        // takes the order away.
        assert_eq!(plan.draw_over(at(7), at(7)), Ok(Drawn::Rubbed));
        assert_eq!(orders(&plan), vec![(0, 2)]);
        // Rubbing out all of one.
        assert_eq!(plan.draw_over(at(0), at(2)), Ok(Drawn::Rubbed));
        assert_eq!(orders(&plan), vec![]);
        // What comes again as well still does, with the order off.
        let both = plan.shuffle_over(at(0), at(3)).unwrap();
        plan.loop_over(at(0), at(3)).unwrap();
        assert_eq!(plan.draw_over(at(1), at(2)), Ok(Drawn::Rubbed));
        assert_eq!(orders(&plan), vec![]);
        assert!(
            plan.span(both)
                .is_some_and(|span| span.is_loop() && !span.is_drawn())
        );
        // What can't be changes nothing: too few steps.
        assert_eq!(plan.draw_over(at(5), at(6)), Err(SpanError::TooFew));
    }

    /// Where a step is now.
    fn pos(plan: &Plan, id: StepId) -> usize {
        plan.position(id).unwrap()
    }

    /// The labels of the rows each section holds, by section.
    fn holds(plan: &Plan, section: StepId) -> Vec<&str> {
        let at = plan.position(section).unwrap();
        plan.steps()[at + 1..plan.group_end(at)]
            .iter()
            .map(|step| step.label.as_str())
            .collect()
    }

    #[test]
    fn a_plan_saved_before_brackets_loads_and_saves_unchanged() {
        // As it is saved, whatever line endings the checkout gave the
        // file: a plan is always written with plain ones.
        let saved = include_str!("../fixtures/plan_before_brackets.json").replace("\r\n", "\n");
        let plan = Plan::from_json(&saved).unwrap();
        assert_eq!(plan.to_json(), saved.trim_end());
        let id = |label: &str| plan.steps().iter().find(|s| s.label == label).unwrap().id;
        // Each section runs to the next no further in, as it did.
        assert_eq!(
            holds(&plan, id("Music")),
            ["Praise", "A", "B", "Reading", "C"]
        );
        assert_eq!(holds(&plan, id("Praise")), ["A", "B"]);
        assert_eq!(holds(&plan, id("Sermon")), ["D"]);
        assert!(plan.steps().iter().all(|step| step.last().is_none()));
        assert_eq!(
            plan.within(plan.position(id("C")).unwrap()),
            [id("Music"), id("Reading")]
        );
        assert_eq!(plan.spans().len(), 2);
    }

    #[test]
    fn a_section_with_its_own_end_stops_while_the_one_around_goes_on() {
        let (mut plan, music, [a, b, c, d, e]) = hymns();
        let inner = plan.bracket_over(c, b, "Duet").unwrap();
        // Its heading went in before B, and it ends at C: D is Music's.
        assert_eq!(
            labels(&plan),
            ["Music", "A", "Duet", "B", "C", "D", "Sermon", "E"]
        );
        assert_eq!(plan.get(inner).and_then(Step::depth), Some(1));
        assert_eq!(plan.get(inner).and_then(Step::last), Some(c));
        assert_eq!(holds(&plan, inner), ["B", "C"]);
        assert_eq!(holds(&plan, music), ["A", "Duet", "B", "C", "D"]);
        assert_eq!(plan.within(pos(&plan, c)), [music, inner]);
        assert_eq!(plan.within(pos(&plan, d)), [music]);
        assert_eq!(plan.level(pos(&plan, d)), 1);
        assert_eq!(plan.level(pos(&plan, c)), 2);
        // What goes on by itself in it is its own to say.
        plan.get_mut(inner).unwrap().advance = Some(true);
        assert!(plan.goes_on(pos(&plan, b)) && plan.goes_on(pos(&plan, c)));
        assert!(!plan.goes_on(pos(&plan, d)) && !plan.goes_on(pos(&plan, a)));
        // Put at its end, a step is not in it; at Music's end it is
        // Music's, which runs on to Sermon.
        assert!(plan.ends_shut(pos(&plan, inner)));
        assert_eq!(plan.level_put_at(pos(&plan, d)), (1, Some(music)));
        assert_eq!(plan.level_put_at(pos(&plan, d) + 1), (1, Some(music)));
        assert_eq!(plan.last_of(inner), Some(c));
        assert_eq!(plan.last_of(music), Some(d));
        // It is saved with its end, and reads back the same.
        let json = plan.to_json();
        assert!(json.contains("\"last\""), "{json}");
        assert_eq!(Plan::from_json(&json).unwrap(), plan);
        // A step at the plan's own level between two sections.
        let sermon = plan.steps()[pos(&plan, e) - 1].id;
        let x = plan.insert(pos(&plan, sermon), "X", Action::Blank);
        assert_eq!(plan.within(pos(&plan, x)), [music], "it ran on to Sermon");
        plan.reach_section(music, Some(d)).unwrap();
        assert!(plan.within(pos(&plan, x)).is_empty());
        assert_eq!(plan.level(pos(&plan, x)), 0);
    }

    #[test]
    fn a_bracket_goes_round_the_sections_wholly_among_its_steps() {
        let (mut plan, [music, praise, reading, sermon]) = nested();
        let at =
            |plan: &Plan, label: &str| plan.steps().iter().find(|s| s.label == label).unwrap().id;
        let (a, c) = (at(&plan, "A"), at(&plan, "C"));
        // From A it would cut Praise in two.
        assert_eq!(
            plan.bracket_over(a, c, "Half"),
            Err(BracketError::HalfOver(praise))
        );
        // Round Praise and Reading: they go one further in.
        let before = plan.clone();
        let round = plan.would_bracket_over(praise, c).unwrap();
        assert_eq!(plan, before, "a dry run changes nothing");
        assert_eq!(plan.bracket_over(praise, c, "Round"), Ok(round));
        assert_eq!(holds(&plan, round), ["Praise", "A", "B", "Reading", "C"]);
        assert_eq!(plan.get(round).and_then(Step::depth), Some(1));
        assert_eq!(plan.get(praise).and_then(Step::depth), Some(DEEPEST));
        assert_eq!(
            plan.within(plan.position(c).unwrap()),
            [music, round, reading]
        );
        // And no deeper than that.
        assert_eq!(
            plan.bracket_over(a, a, "Deeper"),
            Err(BracketError::TooDeep)
        );
        // Ending on a heading, it takes in all that section holds.
        let mut flat = Plan::new("Flat");
        let one = flat.section("One", None);
        let x = flat.push("X", Action::Blank);
        let two = flat.section("Two", None);
        flat.push("Y", Action::Blank);
        // Two beside One: from in One to out of it cuts One in two.
        assert_eq!(
            flat.bracket_over(x, two, "Both"),
            Err(BracketError::HalfOver(one))
        );
        assert!(flat.nest(two, true));
        let both = flat.bracket_over(x, two, "Both").unwrap();
        assert_eq!(holds(&flat, both), ["X", "Two", "Y"]);
        assert_eq!(flat.get(two).and_then(Step::depth), Some(2));
        assert_eq!(flat.within(flat.len() - 1), [one, both, two]);
        let _ = (music, reading, sermon);
    }

    #[test]
    fn a_bracket_that_would_end_a_drawn_order_is_refused() {
        let (mut plan, _, [a, b, c, d, _]) = hymns();
        let drawn = plan.shuffle_over(a, d).unwrap();
        assert_eq!(
            plan.bracket_over(b, c, "In"),
            Err(BracketError::Cuts(drawn))
        );
        assert!(plan.span(drawn).unwrap().flow.shuffle);
        // Round all of it, it is no cut.
        assert!(plan.bracket_over(a, d, "Round").is_ok());
        assert!(plan.span(drawn).unwrap().flow.shuffle);
        // Nor can a section's end come among it.
        let (mut plan, music, [a, b, c, _, _]) = hymns();
        let drawn = plan.shuffle_over(a, c).unwrap();
        assert_eq!(
            plan.reach_section(music, Some(b)),
            Err(BracketError::Cuts(drawn))
        );
        assert_eq!(plan.reach_section(music, Some(c)), Ok(()));
    }

    #[test]
    fn unbracketing_leaves_the_steps_where_they_are() {
        let (mut plan, music, [a, b, c, d, e]) = hymns();
        let sermon = plan.steps()[5].id;
        // The steps of Sermon don't fall into Music: they are the plan's.
        let looped = plan.loop_section(sermon).unwrap();
        let f = plan.push("F", Action::Blank);
        assert!(plan.unbracket(sermon));
        assert_eq!(holds(&plan, music), ["A", "B", "C", "D"]);
        assert!(plan.within(plan.position(e).unwrap()).is_empty());
        // Its loop is a loop over those steps now.
        assert_eq!(
            plan.span(looped).unwrap().over,
            Over::Steps { first: e, last: f }
        );
        // A bracket inside one: its steps stay in the one around it.
        let duet = plan.bracket_over(b, c, "Duet").unwrap();
        assert!(plan.unbracket(duet));
        assert_eq!(holds(&plan, music), ["A", "B", "C", "D"]);
        assert!(!plan.unbracket(a), "a step isn't a section");
        let _ = d;
    }

    #[test]
    fn a_sections_end_is_moved_by_hand_with_the_same_refusals() {
        let (mut plan, music, [a, b, c, d, e]) = hymns();
        let sermon = plan.steps()[5].id;
        // Shorter: C and D are the plan's own.
        plan.reach_section(music, Some(b)).unwrap();
        assert_eq!(holds(&plan, music), ["A", "B"]);
        assert!(plan.within(plan.position(c).unwrap()).is_empty());
        // Longer, round Sermon: it goes one further in.
        plan.reach_section(music, Some(e)).unwrap();
        assert_eq!(holds(&plan, music), ["A", "B", "C", "D", "Sermon", "E"]);
        assert_eq!(plan.get(sermon).and_then(Step::depth), Some(1));
        // Holding nothing: its own heading is its last.
        plan.reach_section(music, Some(music)).unwrap();
        assert!(holds(&plan, music).is_empty());
        assert_eq!(plan.get(sermon).and_then(Step::depth), Some(0));
        // Running on again, to Sermon.
        plan.reach_section(music, None).unwrap();
        assert_eq!(holds(&plan, music), ["A", "B", "C", "D"]);
        assert_eq!(plan.get(music).and_then(Step::last), None);
        // Not before its heading, and not into a part of another.
        let duet = plan.bracket_over(c, d, "Duet").unwrap();
        assert_eq!(
            plan.reach_section(duet, Some(a)),
            Err(BracketError::EndsBefore)
        );
        assert_eq!(
            plan.would_reach_section(music, Some(c)),
            Err(BracketError::HalfOver(duet))
        );
        assert_eq!(
            plan.reach_section(duet, Some(e)),
            Err(BracketError::HalfOver(music))
        );
        assert_eq!(
            plan.reach_section(StepId(99), None),
            Err(BracketError::Missing)
        );
    }

    #[test]
    fn a_section_says_when_it_should_be_over_by() {
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        assert!(!plan.to_json().contains("done_by"));
        if let Action::Section { done_by, .. } = &mut plan.get_mut(music).unwrap().action {
            *done_by = TimeOfDay::parse("11:45");
        }
        let json = plan.to_json();
        assert!(json.contains("\"done_by\": \"11:45\""), "{json}");
        let back = Plan::from_json(&json).unwrap();
        assert_eq!(
            back.get(music).unwrap().done_by(),
            TimeOfDay::parse("11:45")
        );
    }

    #[test]
    fn a_step_carried_out_of_a_loop_is_refused_then_cut_loose() {
        let (mut plan, _, [a, b, c, _, _]) = hymns();
        let sermon = plan.steps()[5].id;
        let looped = plan.loop_over(a, c).unwrap();
        // To the end of Sermon, among the rows without B.
        let end = Place {
            index: plan.len() - 1,
            section: Some(sermon),
        };
        let before = plan.clone();
        assert_eq!(
            plan.carry(&[b], end, Carry::Whole),
            Err(CarryError::Crosses(vec![looped]))
        );
        assert_eq!(plan, before, "nothing changed");
        let carried = plan.carry(&[b], end, Carry::CutLoose).unwrap();
        assert_eq!(carried.steps, [b]);
        assert_eq!(carried.left, [looped]);
        assert!(carried.gone.is_empty());
        assert_eq!(over(&plan, looped), ["A", "C"], "the loop shrank");
        assert_eq!(plan.within(plan.position(b).unwrap()), [sermon]);
        // A loop left with one step is gone.
        let carried = plan.carry(&[c], end, Carry::CutLoose).unwrap();
        assert_eq!(carried.gone, [looped]);
        assert_eq!(plan.span(looped), None);
    }

    #[test]
    fn stripped_steps_leave_their_loops_behind_and_wait() {
        let (mut plan, _, [a, b, c, d, _]) = hymns();
        let sermon = plan.steps()[5].id;
        let outer = plan.loop_over(a, d).unwrap();
        let inner = plan.loop_over(b, c).unwrap();
        plan.get_mut(b).unwrap().advance = Some(true);
        let end = Place {
            index: plan.len() - 2,
            section: Some(sermon),
        };
        // Cut loose, the loop wholly among them goes with them.
        let mut loose = plan.clone();
        let carried = loose.carry(&[c, b], end, Carry::CutLoose).unwrap();
        assert_eq!(carried.steps, [b, c], "in the plan's order");
        assert_eq!(carried.left, [outer]);
        assert_eq!(over(&loose, outer), ["A", "D"]);
        assert_eq!(over(&loose, inner), ["B", "C"]);
        // Stripped, it is taken away and each waits.
        let carried = plan.carry(&[b, c], end, Carry::Stripped).unwrap();
        assert_eq!(carried.left, [outer]);
        assert_eq!(carried.gone, [inner]);
        assert_eq!(plan.span(inner), None);
        assert_eq!(over(&plan, outer), ["A", "D"]);
        assert_eq!(plan.get(b).unwrap().advance, Some(false));
        assert_eq!(plan.get(c).unwrap().advance, Some(false));
    }

    #[test]
    fn joined_steps_keep_their_join_cut_loose_and_lose_it_stripped() {
        let (mut plan, music, [a, b, _, _, _]) = hymns();
        let sermon = plan.steps()[5].id;
        // A goes on to B by itself: Music says so for all its steps.
        plan.get_mut(music).unwrap().advance = Some(true);
        let end = Place {
            index: plan.len() - 2,
            section: Some(sermon),
        };
        let mut loose = plan.clone();
        loose.carry(&[a, b], end, Carry::CutLoose).unwrap();
        let at = loose.position(a).unwrap();
        assert!(loose.goes_on(at), "the join goes along though Sermon waits");
        assert_eq!(loose.get(a).unwrap().advance, Some(true));
        plan.carry(&[a, b], end, Carry::Stripped).unwrap();
        assert!(!plan.goes_on(plan.position(a).unwrap()));
        // Going on by itself never stops a carry.
        let (mut plan, _, [a, _, _, d, _]) = hymns();
        plan.get_mut(a).unwrap().advance = Some(true);
        let to = Place {
            index: plan.position(d).unwrap() - 1,
            section: plan.within(1).first().copied(),
        };
        assert!(plan.carry(&[a], to, Carry::Whole).is_ok());
    }

    #[test]
    fn a_selection_apart_lands_together_in_the_plans_order() {
        let (mut plan, music, [a, b, c, d, e]) = hymns();
        let sermon = plan.steps()[5].id;
        let end = Place {
            index: plan.len() - 2,
            section: Some(sermon),
        };
        plan.carry(&[c, a], end, Carry::Whole).unwrap();
        assert_eq!(labels(&plan), ["Music", "B", "D", "Sermon", "E", "A", "C"]);
        assert_eq!(holds(&plan, sermon), ["E", "A", "C"]);
        // At Music's end, Music or the plan: as asked.
        let at_end = plan.position(sermon).unwrap() - 1;
        assert_eq!(plan.landings(&[e], at_end + 1), [Some(music), None]);
        plan.carry(
            &[e],
            Place {
                index: at_end + 1,
                section: None,
            },
            Carry::Whole,
        )
        .unwrap();
        assert!(plan.within(plan.position(e).unwrap()).is_empty());
        assert_eq!(holds(&plan, music), ["B", "D"]);
        // Not in a section that isn't there.
        assert_eq!(
            plan.carry(
                &[b],
                Place {
                    index: 0,
                    section: Some(sermon)
                },
                Carry::Whole
            ),
            Err(CarryError::Nowhere)
        );
        assert_eq!(
            plan.carry(
                &[music],
                Place {
                    index: 0,
                    section: None
                },
                Carry::Whole
            ),
            Err(CarryError::Heading(music))
        );
        let _ = d;
    }

    #[test]
    fn steps_carried_into_an_empty_section_are_in_it() {
        let mut plan = Plan::new("Sabbath");
        plan.section("Music", None);
        let a = plan.push("A", Action::Blank);
        plan.push("B", Action::Blank);
        let empty = plan.section("Empty", None);
        let x = plan.push("X", Action::Blank);
        plan.reach_section(empty, Some(empty)).unwrap();
        assert!(holds(&plan, empty).is_empty());
        assert!(plan.within(plan.position(x).unwrap()).is_empty());
        // Right after its heading, among the rows without A.
        let index = plan.position(empty).unwrap();
        assert!(plan.landings(&[a], index).contains(&Some(empty)));
        let to = Place {
            index,
            section: Some(empty),
        };
        plan.carry(&[a], to, Carry::Whole).unwrap();
        assert_eq!(holds(&plan, empty), ["A"]);
        assert!(plan.within(plan.position(x).unwrap()).is_empty());
        // A new step put there too.
        let to = Place {
            index: plan.position(a).unwrap() + 1,
            section: Some(empty),
        };
        let new = plan.insert_at(to, "New", Action::Note).unwrap();
        assert_eq!(holds(&plan, empty), ["A", "New"]);
        assert_eq!(plan.get(new).unwrap().label, "New");
    }

    #[test]
    fn moving_about_inside_a_loop_crosses_nothing() {
        let (mut plan, music, [a, _, c, d, _]) = hymns();
        let looped = plan.loop_over(a, d).unwrap();
        // A to between C and D: still in the loop.
        let to = Place {
            index: plan.position(c).unwrap(),
            section: Some(music),
        };
        let carried = plan.carry(&[a], to, Carry::Whole).unwrap();
        assert!(carried.left.is_empty());
        assert_eq!(over(&plan, looped), ["B", "C", "A", "D"]);
        // A loop carried by its own line is never crossed.
        let inner = plan.loop_over(c, a).unwrap();
        let line = plan.span_steps(inner);
        assert_eq!(line, [c, plan.steps()[3].id]);
        let to = Place {
            index: 1,
            section: Some(music),
        };
        // Before B it is out of the outer loop, which is crossed.
        assert_eq!(
            plan.carry(&line, to, Carry::Whole),
            Err(CarryError::Crosses(vec![looped]))
        );
        assert!(plan.carry(&line, to, Carry::CutLoose).is_ok());
        assert_eq!(over(&plan, inner), ["C", "A"]);
        assert_eq!(over(&plan, looped), ["B", "D"]);
    }

    #[test]
    fn a_step_added_after_a_sections_last_stays_in_that_section() {
        let (mut plan, music, [_, b, ..]) = hymns();
        let duet = plan.bracket_over(b, b, "Duet").unwrap();
        let mut session = crate::Session::new(plan);
        let new = session.insert_after(Some(b), "New", Action::Note);
        let plan = session.plan();
        assert_eq!(holds(plan, duet), ["B", "New"]);
        assert_eq!(plan.within(plan.position(new).unwrap()), [music, duet]);
    }

    #[test]
    fn removing_a_section_with_its_own_end_leaves_what_followed_it_alone() {
        let (mut plan, music, [a, b, c, d, _]) = hymns();
        let duet = plan.bracket_over(b, c, "Duet").unwrap();
        plan.remove(duet);
        assert_eq!(holds(&plan, music), ["A", "B", "C", "D"]);
        // A step that was a section's last: it ends at the one before.
        let duet = plan.bracket_over(b, c, "Duet").unwrap();
        plan.remove(c);
        assert_eq!(holds(&plan, duet), ["B"]);
        assert_eq!(plan.within(plan.position(d).unwrap()), [music]);
        let _ = a;
    }

    #[test]
    fn a_loop_drawn_by_hand_joins_one_it_overlaps_and_can_be_rubbed_out_in_part() {
        let mut plan = Plan::new("Sabbath");
        let steps: Vec<StepId> = ["A", "B", "C", "D", "E", "F"]
            .into_iter()
            .map(|name| plan.push(name, Action::Blank))
            .collect();
        let at = |n: usize| steps[n];
        let loops = |plan: &Plan| -> Vec<(usize, usize)> {
            plan.laid_spans()
                .into_iter()
                .filter(|(span, ..)| span.is_loop())
                .map(|(_, first, last)| (first, last))
                .collect()
        };
        let Ok(Looped::Made(first)) = plan.loop_by_hand(at(0), at(2)) else {
            panic!("a loop of its own");
        };
        let mut flow = plan.flow_of(first).unwrap();
        flow.repeat = Repeat::Times(3);
        plan.set_flow(first, flow).unwrap();
        // One inside it is a loop of its own, as before.
        assert!(matches!(
            plan.loop_by_hand(at(0), at(1)),
            Ok(Looped::Made(_))
        ));
        assert_eq!(loops(&plan), vec![(0, 2), (0, 1)]);
        let inner = plan.spans_at(0)[1];
        plan.remove_span(inner);
        // One that takes in a part of it joins it: one loop round all of
        // them, coming as often as the one that was there.
        let Ok(Looped::Grown(grown)) = plan.loop_by_hand(at(2), at(4)) else {
            panic!("it grows");
        };
        assert_eq!(loops(&plan), vec![(0, 4)]);
        assert_eq!(plan.flow_of(grown).unwrap().repeat, Repeat::Times(3));
        // Rubbing its middle out leaves a loop either side, as often.
        assert_eq!(plan.would_rub_loop(grown, at(2), at(2)), Ok(()));
        assert_eq!(loops(&plan), vec![(0, 4)], "a dry run changes nothing");
        plan.rub_loop(grown, at(2), at(2)).unwrap();
        assert_eq!(loops(&plan), vec![(0, 1), (3, 4)]);
        assert!(
            plan.spans()
                .iter()
                .all(|span| span.flow.repeat == Repeat::Times(3))
        );
        // One step left is no loop.
        let left = plan.spans_at(0)[0];
        plan.rub_loop(left, at(1), at(1)).unwrap();
        assert_eq!(loops(&plan), vec![(3, 4)]);
        // Rubbing all of one out.
        let right = plan.spans_at(3)[0];
        plan.rub_loop(right, at(3), at(4)).unwrap();
        assert_eq!(loops(&plan), vec![]);
    }

    #[test]
    fn a_copy_comes_right_after_with_its_flows() {
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        let a = plan.push("A", Action::Blank);
        let b = plan.push("B", Action::Blank);
        let c = plan.push("C", Action::Blank);
        let school = plan.section("School", None);
        let d = plan.push("D", Action::Blank);
        plan.loop_over(a, b).unwrap();
        // A and B, with their loop: the copies after B, in Music, looped.
        let copies = plan.duplicate(&[b, a]);
        assert_eq!(copies.len(), 2);
        assert_eq!(
            labels(&plan),
            ["Music", "A", "B", "A", "B", "C", "School", "D"]
        );
        assert_eq!(plan.within(plan.position(copies[0]).unwrap()), vec![music]);
        let loops: Vec<(usize, usize)> = plan
            .laid_spans()
            .into_iter()
            .filter(|(span, ..)| span.is_loop())
            .map(|(_, first, last)| (first, last))
            .collect();
        assert_eq!(loops, vec![(1, 2), (3, 4)]);
        // A section, with all it holds: the copy right after it.
        let copies = plan.duplicate(&[school]);
        assert_eq!(copies.len(), 1);
        assert_eq!(&labels(&plan)[6..], ["School", "D", "School", "D"]);
        let copy = copies[0];
        let d2 = plan.steps()[9].id;
        assert_eq!(plan.within(plan.position(d2).unwrap()), vec![copy]);
        assert_eq!(plan.within(plan.position(d).unwrap()), vec![school]);
        // Nothing in the plan: nothing copied.
        assert!(plan.duplicate(&[StepId(999)]).is_empty());
        let _ = c;
    }
}
