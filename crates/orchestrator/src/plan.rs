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
    /// Starts a section: the steps after it, up to the next section, are
    /// one part of the service, such as the music or the sermon. Stepping
    /// through the plan passes over it.
    Section {
        /// How long the section should take.
        #[serde(default, with = "seconds", skip_serializing_if = "Option::is_none")]
        budget: Option<Duration>,
        /// How far in it sits: 0 is a part of the plan, 1 a part of the
        /// section before it that sits less far in, and so on to
        /// [`DEEPEST`]. A section holds what follows it, up to the next
        /// section no further in than itself.
        #[serde(default, skip_serializing_if = "is_zero")]
        depth: u8,
    },
}

/// The furthest in a section sits: three levels of section in all.
pub const DEEPEST: u8 = 2;

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(depth: &u8) -> bool {
    *depth == 0
}

/// How many times a step, a section or the plan comes before the plan
/// moves past it.
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
    /// Again while it is before this time of day; what is on when the
    /// time comes is finished first.
    Until(crate::timing::TimeOfDay),
}

/// How a step, a section or the plan moves: how often it comes, and for
/// a section or the plan, in what order its steps do. Unlike
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
    /// How many times it comes, and for a section in what order its
    /// steps do.
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
}

/// An ordered list of steps, prepared ahead of time. Saved as JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// E.g. "Sunday service, March 3".
    pub title: String,
    steps: Vec<Step>,
    #[serde(default)]
    next_id: u64,
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
        let id = StepId(self.next_id);
        self.next_id += 1;
        let step = Step {
            id,
            label: label.into(),
            notes: None,
            action,
            estimate: None,
            advance: None,
            flow: Flow::default(),
            sound: None,
            look: None,
        };
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
        self.push(title, Action::Section { budget, depth: 0 })
    }

    /// Where what the section at `at` holds ends: the next section no
    /// further in than it, or the end of the plan. `at + 1` for a step
    /// that isn't a section.
    pub fn group_end(&self, at: usize) -> usize {
        let Some(depth) = self.steps.get(at).and_then(Step::depth) else {
            return (at + 1).min(self.steps.len());
        };
        self.steps[at + 1..]
            .iter()
            .position(|step| step.depth().is_some_and(|other| other <= depth))
            .map_or(self.steps.len(), |n| at + 1 + n)
    }

    /// How far in the row at `index` is drawn: a section at its depth, a
    /// step one further in than the section it is in.
    pub fn level(&self, index: usize) -> u8 {
        match self.steps.get(index).and_then(Step::depth) {
            Some(depth) => depth,
            None => self.steps[..index.min(self.steps.len())]
                .iter()
                .rev()
                .find_map(Step::depth)
                .map_or(0, |depth| depth + 1),
        }
    }

    /// The sections the step at `index` is in, outermost first. A
    /// section is not in itself.
    pub fn within(&self, index: usize) -> Vec<StepId> {
        let mut found: Vec<(u8, StepId)> = Vec::new();
        let mut under = self
            .steps
            .get(index)
            .and_then(Step::depth)
            .unwrap_or(u8::MAX);
        for step in self.steps[..index.min(self.steps.len())].iter().rev() {
            if let Some(depth) = step.depth()
                && depth < under
            {
                found.push((depth, step.id));
                under = depth;
                if depth == 0 {
                    break;
                }
            }
        }
        found.into_iter().rev().map(|(_, id)| id).collect()
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
    /// the sections in it, with its flow as the plan's and the way its
    /// steps go on as it stood in this plan. For running a part of the
    /// plan by itself. `None` if `id` isn't a section.
    pub fn part(&self, id: StepId) -> Option<Plan> {
        let at = self.position(id)?;
        let depth = self.steps[at].depth()?;
        let mut part = self.clone();
        part.advance = self.goes_on(at);
        part.flow = self.steps[at].flow;
        part.steps = self.steps[at + 1..self.group_end(at)].to_vec();
        for step in &mut part.steps {
            if let Action::Section { depth: inner, .. } = &mut step.action {
                *inner = inner.saturating_sub(depth + 1);
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
        for step in &mut self.steps[at..end] {
            if let Action::Section { depth, .. } = &mut step.action {
                *depth = if inward { *depth + 1 } else { *depth - 1 };
            }
        }
        self.settle();
        true
    }

    /// Keeps every section where it can be: the first at the plan's own
    /// level, and none more than one level further in than the section
    /// before it. Moving or removing steps can leave one stranded.
    fn settle(&mut self) {
        let mut before: Option<u8> = None;
        for step in &mut self.steps {
            if let Action::Section { depth, .. } = &mut step.action {
                let most = before.map_or(0, |before| before + 1).min(DEEPEST);
                *depth = (*depth).min(most);
                before = Some(*depth);
            }
        }
    }

    /// Removes a step.
    pub fn remove(&mut self, id: StepId) -> Option<Step> {
        let index = self.position(id)?;
        // The sections a removed section held move out one level, to
        // where it was, and don't fall into the section before it.
        if let Some(depth) = self.steps[index].depth() {
            let end = self.group_end(index);
            for step in &mut self.steps[index + 1..end] {
                if let Action::Section { depth: inner, .. } = &mut step.action
                    && *inner > depth
                {
                    *inner -= 1;
                }
            }
        }
        let step = self.steps.remove(index);
        self.settle();
        Some(step)
    }

    /// Moves a step to `index` (clamped to the end). Returns whether it exists.
    pub fn move_to(&mut self, id: StepId, index: usize) -> bool {
        let Some(from) = self.position(id) else {
            return false;
        };
        let step = self.steps.remove(from);
        self.steps.insert(index.min(self.steps.len()), step);
        self.settle();
        true
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
    pub fn move_steps(&mut self, ids: &[StepId], index: usize) -> bool {
        let (moving, mut rest): (Vec<Step>, Vec<Step>) = std::mem::take(&mut self.steps)
            .into_iter()
            .partition(|step| ids.contains(&step.id));
        let moved = !moving.is_empty();
        let at = index.min(rest.len());
        rest.splice(at..at, moving);
        self.steps = rest;
        self.settle();
        moved
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
        let mut plan: Plan = serde_json::from_str(json)?;
        // Hand-edited files may have a stale counter; never reuse an id.
        let max = plan.steps.iter().map(|s| s.id.0 + 1).max().unwrap_or(0);
        plan.next_id = plan.next_id.max(max);
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
        plan.get_mut(music).unwrap().flow.repeat = Repeat::Times(2);
        let part = plan.part(music).unwrap();
        assert_eq!(labels(&part), ["Praise", "A", "B", "Reading", "C"]);
        // The sections in it are its own sections now.
        assert_eq!(part.get(praise).and_then(Step::depth), Some(0));
        // It moves as Music did.
        assert!(part.advance);
        assert_eq!(part.flow.repeat, Repeat::Times(2));
        assert!(part.goes_on(1));
        // The same steps, by the same ids.
        assert_eq!(part.steps()[1].id, plan.steps()[2].id);
        assert_eq!(labels(&plan.part(sermon).unwrap()), ["D"]);
        assert!(plan.part(part.steps()[1].id).is_none(), "a step isn't one");
    }
}
