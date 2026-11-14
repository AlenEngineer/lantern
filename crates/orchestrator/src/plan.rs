use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lantern_resource::{Query, Resource};
use lantern_showman::Look;
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
pub struct StepId(u64);

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
    },
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
    /// anything else after its length. Off, the operator moves on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub advance: bool,
}

impl Step {
    /// Whether this step starts a section rather than doing something.
    pub fn is_section(&self) -> bool {
        matches!(self.action, Action::Section { .. })
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
            advance: false,
        };
        self.steps.insert(index.min(self.steps.len()), step);
        id
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
        self.push(title, Action::Section { budget })
    }

    /// Removes a step.
    pub fn remove(&mut self, id: StepId) -> Option<Step> {
        let index = self.position(id)?;
        Some(self.steps.remove(index))
    }

    /// Moves a step to `index` (clamped to the end). Returns whether it exists.
    pub fn move_to(&mut self, id: StepId, index: usize) -> bool {
        let Some(from) = self.position(id) else {
            return false;
        };
        let step = self.steps.remove(from);
        self.steps.insert(index.min(self.steps.len()), step);
        true
    }

    /// What moving `id` takes along: the step alone, or a section with
    /// every step up to the next section. Empty if there's no such step.
    pub fn carried(&self, id: StepId) -> Vec<StepId> {
        let Some(at) = self.position(id) else {
            return Vec::new();
        };
        let mut ids = vec![id];
        if self.steps[at].is_section() {
            ids.extend(
                self.steps[at + 1..]
                    .iter()
                    .take_while(|step| !step.is_section())
                    .map(|step| step.id),
            );
        }
        ids
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
        template.get_mut(welcome).unwrap().advance = true;
        if let Action::Live { shown, .. } = &mut template.get_mut(slot).unwrap().action {
            shown.push(resource("Himno 436"));
        }
        let json = template.to_json();
        assert!(json.contains("\"template\": true"), "{json}");

        let sunday = template.from_template("Domingo 2026-11-15");
        assert_eq!(sunday.title, "Domingo 2026-11-15");
        assert!(!sunday.template);
        assert_eq!(labels(&sunday), labels(&template));
        assert!(sunday.get(welcome).unwrap().advance, "settings come along");
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
}
