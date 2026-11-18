//! Notes for the operator, on what they are about: a step, a section,
//! some steps together, or a loop or drawn order. One note can be on
//! several things; it is shown on each of them, and goes when all it is
//! on has gone.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::plan::{Plan, SpanId, StepId};

/// Identifies a note within its plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NoteId(pub(crate) u64);

impl fmt::Display for NoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "†{}", self.0)
    }
}

/// What a note is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteOn {
    /// A step, or a section by its heading.
    Step(StepId),
    /// A loop or a drawn order.
    Span(SpanId),
}

/// A note: what it says, and what it is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// Stable id.
    pub id: NoteId,
    /// What it says.
    pub text: String,
    /// What it is on, each once.
    pub on: Vec<NoteOn>,
}

impl Plan {
    /// Every note, in the order they were made.
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// A note by id.
    pub fn note(&self, id: NoteId) -> Option<&Note> {
        self.notes.iter().find(|note| note.id == id)
    }

    /// The notes on `on`.
    pub fn notes_on(&self, on: NoteOn) -> Vec<&Note> {
        self.notes
            .iter()
            .filter(|note| note.on.contains(&on))
            .collect()
    }

    /// Whether `on` is in the plan.
    fn holds(&self, on: NoteOn) -> bool {
        match on {
            NoteOn::Step(id) => self.position(id).is_some(),
            NoteOn::Span(id) => self.span(id).is_some(),
        }
    }

    /// A new note saying `text`, on what of `on` is in the plan. `None`
    /// where none of it is.
    pub fn add_note(&mut self, on: &[NoteOn], text: impl Into<String>) -> Option<NoteId> {
        let mut kept: Vec<NoteOn> = Vec::new();
        for at in on {
            if self.holds(*at) && !kept.contains(at) {
                kept.push(*at);
            }
        }
        if kept.is_empty() {
            return None;
        }
        let id = NoteId(self.next_note);
        self.next_note += 1;
        self.notes.push(Note {
            id,
            text: text.into(),
            on: kept,
        });
        Some(id)
    }

    /// Has the note `id` say `text`. Whether there is such a note.
    pub fn set_note_text(&mut self, id: NoteId, text: impl Into<String>) -> bool {
        match self.notes.iter_mut().find(|note| note.id == id) {
            Some(note) => {
                note.text = text.into();
                true
            }
            None => false,
        }
    }

    /// Puts the note `id` on `on` as well. Whether it is now on it.
    pub fn attach_note(&mut self, id: NoteId, on: NoteOn) -> bool {
        if !self.holds(on) {
            return false;
        }
        match self.notes.iter_mut().find(|note| note.id == id) {
            Some(note) => {
                if !note.on.contains(&on) {
                    note.on.push(on);
                }
                true
            }
            None => false,
        }
    }

    /// Takes the note `id` away. Returns it, if it was there.
    pub fn remove_note(&mut self, id: NoteId) -> Option<Note> {
        let at = self.notes.iter().position(|note| note.id == id)?;
        Some(self.notes.remove(at))
    }

    /// The notes with the number each is shown with, from 1: in the order
    /// of the first thing each is on, down the plan.
    pub fn numbered_notes(&self) -> Vec<(NoteId, usize)> {
        let first = |note: &Note| {
            note.on
                .iter()
                .filter_map(|on| match on {
                    NoteOn::Step(id) => self.position(*id),
                    NoteOn::Span(id) => self.span_range(*id).map(|(first, _)| first),
                })
                .min()
                .unwrap_or(usize::MAX)
        };
        let mut found: Vec<(usize, NoteId)> = self
            .notes
            .iter()
            .map(|note| (first(note), note.id))
            .collect();
        found.sort_unstable();
        found
            .into_iter()
            .enumerate()
            .map(|(n, (_, id))| (id, n + 1))
            .collect()
    }

    /// Drops from each note what is no longer in the plan, and the notes
    /// that are on nothing then.
    pub(crate) fn keep_notes(&mut self) {
        let mut notes = std::mem::take(&mut self.notes);
        for note in &mut notes {
            note.on.retain(|on| self.holds(*on));
        }
        notes.retain(|note| !note.on.is_empty());
        self.notes = notes;
    }

    /// The notes a plan saved before there were notes kept on its steps
    /// (`Step::notes`), as notes on those steps.
    pub(crate) fn upgrade_step_notes(&mut self) {
        let found: Vec<(StepId, String)> = self
            .steps()
            .iter()
            .filter_map(|step| {
                let text = step.notes.clone()?;
                (!text.trim().is_empty()).then_some((step.id, text))
            })
            .collect();
        for (id, text) in found {
            self.add_note(&[NoteOn::Step(id)], text);
            if let Some(step) = self.get_mut(id) {
                step.notes = None;
            }
        }
        let max = self
            .notes
            .iter()
            .map(|note| note.id.0 + 1)
            .max()
            .unwrap_or(0);
        self.next_note = self.next_note.max(max);
    }
}

#[cfg(test)]
mod tests {
    use crate::plan::Action;

    use super::*;

    #[test]
    fn a_note_is_on_what_it_is_about_and_goes_with_it() {
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        let a = plan.push("A", Action::Blank);
        let b = plan.push("B", Action::Blank);
        let c = plan.push("C", Action::Blank);
        let span = plan.loop_over(a, b).unwrap();
        let both = plan
            .add_note(&[NoteOn::Step(b), NoteOn::Step(a)], "Lights down")
            .unwrap();
        let round = plan
            .add_note(&[NoteOn::Span(span)], "Twice, slower")
            .unwrap();
        let all = plan.add_note(&[NoteOn::Step(music)], "Stand up").unwrap();
        assert_eq!(plan.notes_on(NoteOn::Step(a)).len(), 1);
        assert_eq!(plan.notes_on(NoteOn::Step(c)).len(), 0);
        // Numbered down the plan by the first thing each is on: Music,
        // then A (the note on A and B, and the loop from A).
        let numbers = plan.numbered_notes();
        assert_eq!(numbers[0], (all, 1));
        assert!(numbers.contains(&(both, 2)) || numbers.contains(&(both, 3)));
        // Nothing that isn't in the plan.
        assert_eq!(plan.add_note(&[NoteOn::Step(StepId(99))], "x"), None);
        // Put on C too, then A and B taken out: it stays, on C.
        assert!(plan.attach_note(both, NoteOn::Step(c)));
        plan.remove(a);
        plan.remove(b);
        plan.keep_notes();
        assert_eq!(plan.note(both).unwrap().on, vec![NoteOn::Step(c)]);
        // The loop went with its steps, and its note with it.
        assert!(plan.note(round).is_none());
        assert!(plan.set_note_text(all, "Sit down"));
        assert_eq!(plan.note(all).unwrap().text, "Sit down");
        assert!(plan.remove_note(all).is_some());
    }

    #[test]
    fn a_plan_saved_with_notes_on_its_steps_keeps_them_as_notes() {
        let mut plan = Plan::new("Sabbath");
        let a = plan.push("A", Action::Blank);
        plan.get_mut(a).unwrap().notes = Some("Mic 2".into());
        let again = Plan::from_json(&plan.to_json()).unwrap();
        let notes = again.notes_on(NoteOn::Step(a));
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].text, "Mic 2");
        assert!(again.get(a).unwrap().notes.is_none());
        // And notes are kept as they are.
        let mut kept = again.clone();
        kept.add_note(&[NoteOn::Step(a)], "Mic 3");
        let back = Plan::from_json(&kept.to_json()).unwrap();
        assert_eq!(back.notes_on(NoteOn::Step(a)).len(), 2);
    }
}
