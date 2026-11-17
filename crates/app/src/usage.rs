//! What has been shown, and when: so that filling a section can prefer
//! what is usually used and leave out what was on lately.
//!
//! One line a day for each thing the operator picked and put on the
//! projector, in a service or in Live, kept in `usage` in the data
//! folder: its id and the day. Looking at something while planning is
//! not using it, and neither is showing a step that a fill put in the
//! plan: only what was chosen by hand says what is usually used. It
//! stays on this machine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use lantern_engine::ResourceId;

/// The days each thing was used on, oldest first.
pub struct Usage {
    file: PathBuf,
    days: HashMap<String, Vec<NaiveDate>>,
}

fn read(text: &str) -> HashMap<String, Vec<NaiveDate>> {
    let mut days: HashMap<String, Vec<NaiveDate>> = HashMap::new();
    for line in text.lines() {
        let Some((id, day)) = line.rsplit_once('\t') else {
            continue;
        };
        let Ok(day) = day.parse::<NaiveDate>() else {
            continue;
        };
        let of = days.entry(id.to_string()).or_default();
        if !of.contains(&day) {
            of.push(day);
        }
    }
    for of in days.values_mut() {
        of.sort_unstable();
    }
    days
}

impl Usage {
    pub fn read(data: &Path) -> Usage {
        let file = data.join("usage");
        let days = read(&std::fs::read_to_string(&file).unwrap_or_default());
        Usage { file, days }
    }

    /// `id` was on the projector on `day`. Once a day is enough.
    pub fn note(&mut self, id: &ResourceId, day: NaiveDate) {
        let of = self.days.entry(id.to_string()).or_default();
        if of.contains(&day) {
            return;
        }
        of.push(day);
        of.sort_unstable();
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let kept = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)
            .and_then(|mut file| {
                std::io::Write::write_all(&mut file, format!("{id}\t{day}\n").as_bytes())
            });
        if let Err(error) = kept {
            crate::log(format!("usage: not kept: {error}"));
        }
    }

    /// On how many days `id` has been used.
    pub fn times(&self, id: &ResourceId) -> usize {
        self.days.get(&id.to_string()).map_or(0, Vec::len)
    }

    /// How many whole weeks before `today` it was last used.
    pub fn weeks_since(&self, id: &ResourceId, today: NaiveDate) -> Option<i64> {
        let last = *self.days.get(&id.to_string())?.last()?;
        Some((today - last).num_days().max(0) / 7)
    }
}

#[cfg(test)]
mod tests {
    use lantern_engine::SourceId;

    use super::*;

    #[test]
    fn a_thing_is_used_once_a_day_and_kept_between_runs() {
        let dir = tempfile::tempdir().unwrap();
        let source = SourceId::new("local").unwrap();
        let hymn = ResourceId::new(&source, "Himno 99");
        let other = ResourceId::new(&source, "Himno 12");
        let day = |text: &str| text.parse::<NaiveDate>().unwrap();
        let mut usage = Usage::read(dir.path());
        assert_eq!(usage.times(&hymn), 0);
        assert_eq!(usage.weeks_since(&hymn, day("2026-11-21")), None);
        usage.note(&hymn, day("2026-11-07"));
        usage.note(&hymn, day("2026-11-07"));
        usage.note(&hymn, day("2026-10-10"));
        usage.note(&other, day("2026-11-14"));
        assert_eq!(usage.times(&hymn), 2);
        assert_eq!(usage.weeks_since(&hymn, day("2026-11-21")), Some(2));
        assert_eq!(usage.weeks_since(&other, day("2026-11-21")), Some(1));
        // Read again: the same.
        let again = Usage::read(dir.path());
        assert_eq!(again.times(&hymn), 2);
        assert_eq!(again.weeks_since(&hymn, day("2026-11-21")), Some(2));
        assert_eq!(again.times(&other), 1);
    }
}
