//! Plan files: the folder plans are saved in, which file the open plan
//! belongs to, and whether it has changes that aren't on disk.
//!
//! Plans live in `plans/` under [`config::xdg_dir`](crate::config::xdg_dir)
//! for `XDG_DATA_HOME` (`~/.local/share/lantern`), named after their title.
//! `last-plan` next to it holds the path of the plan last opened or saved,
//! which the app reopens at startup.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use lantern_engine::{Engine, Plan};

/// The data folder: where plans are saved and the last one is remembered.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
    last: PathBuf,
}

/// A saved plan, as the open list shows it.
#[derive(Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub modified: SystemTime,
    /// What the plan is, or why the file can't be read.
    pub summary: Result<Summary, String>,
}

/// A saved plan, as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub title: String,
    /// Its steps, not counting section headings: what the plan's header
    /// counts once it's open.
    pub steps: usize,
    /// It's an order of service to start new plans from.
    pub template: bool,
}

impl Store {
    pub fn new(data: &Path) -> Store {
        Store {
            dir: data.join("plans"),
            last: data.join("last-plan"),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every `.json` file in the folder, newest first. A missing folder is empty.
    pub fn list(&self) -> io::Result<Vec<Entry>> {
        let files = match std::fs::read_dir(&self.dir) {
            Ok(files) => files,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let mut entries = Vec::new();
        for file in files {
            let path = file?.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                entries.push(Entry {
                    modified: std::fs::metadata(&path)?.modified()?,
                    summary: Plan::load(&path)
                        .map(|plan| Summary {
                            title: plan.title.clone(),
                            steps: plan.steps().iter().filter(|s| !s.is_section()).count(),
                            template: plan.template,
                        })
                        .map_err(|err| err.to_string()),
                    path,
                });
            }
        }
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.modified));
        Ok(entries)
    }

    /// Makes the saved plan at `path` a template, or not.
    pub fn set_template(&self, path: &Path, template: bool) -> io::Result<()> {
        let mut plan = Plan::load(path).map_err(io::Error::other)?;
        plan.template = template;
        self.write(path, &plan)
    }

    /// A file for a new plan with this title that no other plan uses:
    /// `slug.json`, else `slug-2.json`, `slug-3.json`...
    fn free_path(&self, title: &str) -> PathBuf {
        let slug = slug(title);
        (1..)
            .map(|n| match n {
                1 => self.dir.join(format!("{slug}.json")),
                n => self.dir.join(format!("{slug}-{n}.json")),
            })
            .find(|path| !path.exists())
            .expect("some numbered name is free")
    }

    /// Writes beside the file and renames over it, so a crash mid-write
    /// never leaves half a plan.
    fn write(&self, path: &Path, plan: &Plan) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let partial = path.with_extension("json.partial");
        std::fs::write(&partial, plan.to_json())?;
        std::fs::rename(&partial, path)
    }

    /// Only called for a file in the folder, so the data folder exists.
    fn remember(&self, path: &Path) -> io::Result<()> {
        std::fs::write(&self.last, path.as_os_str().as_encoded_bytes())
    }

    fn last(&self) -> Option<PathBuf> {
        let path = std::fs::read_to_string(&self.last).ok()?;
        Some(PathBuf::from(path.trim_end_matches('\n')))
    }
}

/// Today's date, `2026-11-15`, in local time: the system's `date` when it
/// has one, else UTC.
fn today() -> String {
    let local = std::process::Command::new("date")
        .arg("+%F")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|day| day.trim().to_string())
        .filter(|day| day.len() == 10);
    local.unwrap_or_else(|| {
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let (year, month, day) = civil(i64::try_from(secs / 86_400).unwrap_or(0));
        format!("{year:04}-{month:02}-{day:02}")
    })
}

/// The calendar date `days` after 1970-01-01 (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// A file name from a title: lowercase letters and digits, dashes between
/// words. Accented letters stay.
pub fn slug(title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        "plan".into()
    } else {
        words.join("-")
    }
}

/// What replaces the open plan.
#[derive(Debug)]
pub enum Switch {
    New,
    Open(PathBuf),
    /// A new plan from the template saved at this path.
    FromTemplate(PathBuf),
}

/// What the plan panel shows besides the steps.
#[derive(Debug)]
pub enum Mode {
    Steps,
    /// The name is being edited; the draft so far.
    Renaming(String),
    /// The list of saved plans to open.
    Browsing(Vec<Entry>),
    /// The plan has unsaved changes and no file; asks before discarding them.
    Confirm(Switch),
}

/// The outcome of the last save, open or startup, for the operator.
#[derive(Debug, PartialEq, Eq)]
pub enum Notice {
    Done(String),
    Failed(String),
}

impl Notice {
    fn couldnt(verb: &str, path: &Path, err: impl std::fmt::Display) -> Notice {
        Notice::Failed(format!("Couldn't {verb} {}: {err}", path.display()))
    }
}

/// The open plan's file. The engine holds the plan itself.
pub struct Plans {
    store: Store,
    file: Option<PathBuf>,
    /// The plan as it is on disk, or as it started if it has no file.
    saved: Plan,
    pub mode: Mode,
    pub notice: Option<Notice>,
}

impl Plans {
    /// Reopens the plan last opened or saved. If it can't, the engine's
    /// plan stays and the notice says why.
    pub fn start(store: Store, engine: &Engine) -> Plans {
        let mut plans = Plans {
            store,
            file: None,
            saved: engine.plan(),
            mode: Mode::Steps,
            notice: None,
        };
        if let Some(path) = plans.store.last() {
            plans.load(engine, Switch::Open(path));
        }
        plans
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Whether `plan`, the engine's plan, differs from what was last saved or opened.
    pub fn is_dirty(&self, plan: &Plan) -> bool {
        *plan != self.saved
    }

    /// Whether the plan was saved to or opened from a file.
    pub fn has_file(&self) -> bool {
        self.file.is_some()
    }

    /// Saves to the plan's file, or to a new one named after its title.
    /// A name being edited is applied first. Returns whether it saved.
    pub fn save(&mut self, engine: &Engine) -> bool {
        self.submit_name(engine);
        let plan = engine.plan();
        let path = self
            .file
            .clone()
            .unwrap_or_else(|| self.store.free_path(&plan.title));
        if let Err(err) = self.store.write(&path, &plan) {
            self.notice = Some(Notice::couldnt("save", &path, err));
            return false;
        }
        // The plan's name, not its file's: errors are where a path helps.
        self.notice = Some(self.remembered(&path, format!("Saved “{}”", plan.title)));
        self.file = Some(path);
        self.saved = plan;
        true
    }

    /// Reads the list of saved plans again, when it's showing.
    pub fn refresh(&mut self) {
        if matches!(self.mode, Mode::Browsing(_)) {
            match self.store.list() {
                Ok(entries) => self.mode = Mode::Browsing(entries),
                Err(err) => self.notice = Some(Notice::couldnt("list", &self.store.dir, err)),
            }
        }
    }

    /// Shows the list of saved plans, or hides it.
    pub fn browse(&mut self) {
        if matches!(self.mode, Mode::Browsing(_)) {
            self.mode = Mode::Steps;
            return;
        }
        match self.store.list() {
            Ok(entries) => self.mode = Mode::Browsing(entries),
            Err(err) => self.notice = Some(Notice::couldnt("list", &self.store.dir, err)),
        }
    }

    /// Replaces the plan. Unsaved changes are saved first if the plan has
    /// a file, and otherwise need [`discard`](Self::discard). Returns
    /// whether the engine got a new plan.
    pub fn switch(&mut self, engine: &Engine, to: Switch) -> bool {
        if self.is_dirty(&engine.plan()) {
            if self.file.is_none() {
                self.mode = Mode::Confirm(to);
                return false;
            }
            if !self.save(engine) {
                return false;
            }
        }
        self.load(engine, to)
    }

    /// The saved plan with this title or file name.
    pub fn find(&self, name: &str) -> Option<PathBuf> {
        self.store.list().ok()?.into_iter().find_map(|entry| {
            let named = entry.path.file_stem().is_some_and(|stem| stem == name)
                || entry
                    .summary
                    .as_ref()
                    .is_ok_and(|summary| summary.title.eq_ignore_ascii_case(name));
            named.then_some(entry.path)
        })
    }

    /// Opens the saved plan with this title or file name.
    pub fn open_named(&mut self, engine: &Engine, name: &str) -> bool {
        let found = self.store.list().ok().and_then(|entries| {
            entries.into_iter().find(|entry| {
                entry.path.file_stem().is_some_and(|stem| stem == name)
                    || entry
                        .summary
                        .as_ref()
                        .is_ok_and(|summary| summary.title.eq_ignore_ascii_case(name))
            })
        });
        match found {
            Some(entry) => self.switch(engine, Switch::Open(entry.path)),
            None => {
                self.notice = Some(Notice::Failed(format!("No saved plan named “{name}”")));
                false
            }
        }
    }

    /// Goes ahead with the switch that was waiting on [`Mode::Confirm`].
    pub fn discard(&mut self, engine: &Engine) -> bool {
        match std::mem::replace(&mut self.mode, Mode::Steps) {
            Mode::Confirm(to) => self.load(engine, to),
            _ => false,
        }
    }

    pub fn edit_name(&mut self, engine: &Engine) {
        self.mode = Mode::Renaming(engine.plan().title);
    }

    pub fn type_name(&mut self, text: String) {
        if let Mode::Renaming(draft) = &mut self.mode {
            *draft = text;
        }
    }

    /// Applies the name being edited, if any, and closes the editor.
    pub fn submit_name(&mut self, engine: &Engine) {
        if let Mode::Renaming(draft) = std::mem::replace(&mut self.mode, Mode::Steps) {
            let name = draft.trim();
            if !name.is_empty() {
                engine.edit_plan(|plan| plan.title = name.to_string());
            }
        }
    }

    pub fn cancel(&mut self) {
        self.mode = Mode::Steps;
    }

    fn load(&mut self, engine: &Engine, to: Switch) -> bool {
        self.mode = Mode::Steps;
        let (file, plan, notice) = match to {
            Switch::New => (None, Plan::default(), None),
            // Unsaved, so it shows Save and asks before it's replaced.
            Switch::FromTemplate(path) => match Plan::load(&path) {
                Ok(template) => {
                    let plan = template.from_template(format!("{} {}", template.title, today()));
                    engine.load_plan(plan);
                    self.file = None;
                    self.saved = Plan::default();
                    self.notice = Some(Notice::Done(format!("New plan from {}", template.title)));
                    return true;
                }
                Err(err) => {
                    self.notice = Some(Notice::couldnt("open", &path, err));
                    return false;
                }
            },
            Switch::Open(path) => match Plan::load(&path) {
                Ok(plan) => {
                    let notice = self.remembered(&path, format!("Opened “{}”", plan.title));
                    (Some(path), plan, Some(notice))
                }
                Err(err) => {
                    self.notice = Some(Notice::couldnt("open", &path, err));
                    return false;
                }
            },
        };
        engine.load_plan(plan.clone());
        self.file = file;
        self.saved = plan;
        self.notice = notice;
        true
    }

    /// `done`, unless the path couldn't be remembered for the next start.
    fn remembered(&self, path: &Path, done: String) -> Notice {
        match self.store.remember(path) {
            Ok(()) => Notice::Done(done),
            Err(err) => Notice::Failed(format!("{done}, but couldn't remember it: {err}")),
        }
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use lantern_engine::Action;

    use super::*;

    fn setup() -> (tempfile::TempDir, Store, Engine) {
        let data = tempfile::tempdir().unwrap();
        let store = Store::new(data.path());
        (data, store, Engine::new())
    }

    fn rename(plans: &mut Plans, engine: &Engine, name: &str) {
        plans.edit_name(engine);
        plans.type_name(name.into());
        plans.submit_name(engine);
    }

    fn add_step(engine: &Engine) {
        engine.edit_plan(|plan| plan.push("Prayer", Action::Blank));
    }

    #[test]
    fn slugs_keep_words_and_letters() {
        assert_eq!(slug("Sunday service, March 3"), "sunday-service-march-3");
        assert_eq!(slug("  Oración del Día "), "oración-del-día");
        assert_eq!(slug("!!!"), "plan");
    }

    #[test]
    fn a_template_starts_an_unsaved_plan_dated_today() {
        let (_data, store, engine) = setup();
        let mut plans = Plans::start(store.clone(), &engine);
        rename(&mut plans, &engine, "Domingo");
        engine.edit_plan(|plan| plan.section("Alabanza", None));
        add_step(&engine);
        assert!(plans.save(&engine));
        let path = plans.file.clone().unwrap();

        store.set_template(&path, true).unwrap();
        let listed = store.list().unwrap();
        assert!(listed[0].summary.as_ref().unwrap().template);
        assert_eq!(
            listed[0].summary.as_ref().unwrap().steps,
            1,
            "a section heading isn't a step, as in the plan's header"
        );

        assert!(plans.switch(&engine, Switch::FromTemplate(path.clone())));
        let plan = engine.plan();
        assert_eq!(plan.title, format!("Domingo {}", today()));
        assert!(!plan.template);
        assert_eq!(plan.len(), 2, "the section and the step");
        assert!(plans.is_dirty(&plan), "unsaved, so it shows Save");
        assert_eq!(
            plans.file, None,
            "saving makes a new file, never the template"
        );
        assert!(
            Plan::load(&path).unwrap().template,
            "the template is untouched"
        );
        assert_eq!(today().len(), 10);
    }

    #[test]
    fn dates_count_from_1970() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_407), (2025, 11, 15));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn a_new_plan_never_overwrites_another_with_the_same_name() {
        let (_data, store, engine) = setup();
        let mut plans = Plans::start(store.clone(), &engine);
        let mut saved = Vec::new();
        for _ in 0..3 {
            plans.load(&engine, Switch::New);
            rename(&mut plans, &engine, "Sunday service");
            add_step(&engine);
            assert!(plans.save(&engine));
            saved.push(plans.file.clone().unwrap());
        }
        let names: Vec<_> = saved.iter().map(|path| file_name(path)).collect();
        assert_eq!(
            names,
            [
                "sunday-service.json",
                "sunday-service-2.json",
                "sunday-service-3.json"
            ]
        );

        add_step(&engine);
        assert!(
            plans.save(&engine),
            "an opened plan saves over its own file"
        );
        assert_eq!(store.list().unwrap().len(), 3);
    }

    #[test]
    fn edits_are_unsaved_until_saved() {
        let (_data, store, engine) = setup();
        let mut plans = Plans::start(store, &engine);
        assert!(
            !plans.is_dirty(&engine.plan()),
            "a fresh plan has nothing to lose"
        );
        add_step(&engine);
        assert!(plans.is_dirty(&engine.plan()));
        plans.save(&engine);
        assert!(!plans.is_dirty(&engine.plan()));
        rename(&mut plans, &engine, "  ");
        assert!(!plans.is_dirty(&engine.plan()), "a blank name is ignored");
        rename(&mut plans, &engine, "Evening");
        assert_eq!(engine.plan().title, "Evening");
        assert!(plans.is_dirty(&engine.plan()));
    }

    #[test]
    fn switching_from_an_unsaved_plan_asks_and_from_a_saved_one_saves() {
        let (_data, store, engine) = setup();
        let mut plans = Plans::start(store, &engine);
        add_step(&engine);
        assert!(!plans.switch(&engine, Switch::New));
        assert!(matches!(plans.mode, Mode::Confirm(Switch::New)));
        assert!(plans.discard(&engine));
        assert!(engine.plan().is_empty());

        add_step(&engine);
        plans.save(&engine);
        let file = plans.file.clone().unwrap();
        add_step(&engine);
        assert!(plans.switch(&engine, Switch::New));
        assert_eq!(
            Plan::load(&file).unwrap().len(),
            2,
            "autosaved before switching"
        );
    }

    #[test]
    fn startup_reopens_the_last_plan_or_says_why_not() {
        let (_data, store, engine) = setup();
        let mut plans = Plans::start(store.clone(), &engine);
        assert_eq!(plans.notice, None, "nothing to reopen the first time");
        rename(&mut plans, &engine, "Sunday");
        add_step(&engine);
        plans.save(&engine);

        let engine = Engine::new();
        let mut plans = Plans::start(store.clone(), &engine);
        assert_eq!(engine.plan().title, "Sunday");
        assert!(!plans.is_dirty(&engine.plan()));
        assert!(plans.open_named(&engine, "sunday"), "by file name");

        std::fs::write(plans.file.as_ref().unwrap(), "{ not a plan").unwrap();
        let engine = Engine::new();
        let plans = Plans::start(store, &engine);
        assert_eq!(engine.plan(), Plan::default());
        assert!(
            matches!(&plans.notice, Some(Notice::Failed(why)) if why.contains("invalid plan")),
            "{:?}",
            plans.notice
        );
    }
}
