//! The settings view (Ctrl+,): what `config.toml` holds, as fields. Save
//! writes the file and applies it at once: the library swaps its sources
//! and reindexes, and the colours reload. Path fields suggest what's on
//! disk as they're typed (see [`crate::paths`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use iced::widget::{Space, button, column, row, text, text_input};
use iced::{Alignment, Element, Fill};

use lantern_engine::{Look, Plan, Rgb, fold};

use crate::config::{self, BibleEntry, Config, FolderEntry, Lang};
use crate::dashboard::{chip, pill, scrolling, wide};
use crate::fit;
use crate::keys::{self, Keymap, Op};
use crate::library;
use crate::look::{self, BODY, BOLD, HEADING, Icon, MEDIUM, META, PILL, Tone};
use crate::outputs::Output;
use crate::palette::Palette;
use crate::paths::{self, Suggestion, Want};
use crate::{App, Message};

const LABEL_FIELD: f32 = 160.0;
const SMALL_FIELD: f32 = 110.0;
const BANG_FIELD: f32 = 80.0;
/// The settings' scrolling body.
pub const SCROLL: &str = "settings-body";
const SAMPLE_WIDTH: f32 = 384.0;
/// The list of the settings' parts, and the most a part's fields spread.
const CATEGORIES: f32 = 132.0;
const PART_WIDTH: f32 = 640.0;
/// How tall the small picture of what's on stage is, in the header.
const LIVE_HEIGHT: f32 = 54.0;

/// The settings as they're being edited.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    folders: Vec<Folder>,
    bibles: Vec<Bible>,
    brave_key: String,
    colors: String,
    /// The screen for the stage; `None` leaves it to the compositor.
    pub output: Option<String>,
    /// The screens connected when the settings opened, or why they can't
    /// be listed.
    outputs: Result<Vec<Output>, String>,
    /// The look for all plans.
    stage: Stage,
    /// The open plan's look, when the settings were opened with a plan.
    plan: Option<PlanLook>,
    /// Which look the stage fields edit.
    pub scope: Scope,
    /// The part of the settings on show.
    pub category: Category,
    /// The action of `keys::ACTIONS` waiting for its new key to be pressed.
    pub recording: Option<usize>,
    /// The font families installed, for the font field to suggest.
    fonts: Vec<String>,
    /// What the last save did.
    pub notice: Option<Result<String, String>>,
    home: Option<PathBuf>,
    /// The path field being typed in, and what it could become.
    suggesting: Option<(Field, Vec<Suggestion>)>,
    /// The key of each of `keys::ACTIONS`, as typed (`ctrl+n`).
    keys: Vec<String>,
    /// What the fields held when they were last saved or opened, to tell
    /// whether closing would lose anything.
    saved: String,
    /// Closing was asked for with changes unsaved: asking again closes.
    pub leaving: bool,
}

/// A part of the settings: one shows at a time, picked from the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// The open plan: its name, template, copy, delete, and its own look.
    Plan,
    /// The look for all plans.
    Stage,
    Output,
    /// Media folders, Bibles and image search.
    Library,
    /// The dashboard's colours.
    Appearance,
    Keys,
}

impl Category {
    fn name(self) -> &'static str {
        match self {
            Category::Plan => "This plan",
            Category::Stage => "Stage",
            Category::Output => "Output",
            Category::Library => "Library",
            Category::Appearance => "Appearance",
            Category::Keys => "Keys",
        }
    }
}

/// Which look the stage section edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The look every plan uses unless it has its own.
    AllPlans,
    /// The open plan's own look.
    ThisPlan,
}

/// The open plan's look: its title, and its own look if it has one.
#[derive(Debug, Clone, PartialEq)]
struct PlanLook {
    title: String,
    own: Option<Stage>,
    /// The folders only this plan uses, as typed.
    folders: Vec<String>,
    /// The Bibles it searches, by name; empty for all of them.
    bibles: Vec<String>,
}

/// The stage's look, as typed.
#[derive(Debug, Clone, PartialEq)]
struct Stage {
    background: String,
    image: String,
    text: String,
    font: String,
    max_size: String,
    min_size: String,
    dim: String,
}

impl Stage {
    fn of(look: &Look, shown: &dyn Fn(&Path) -> String) -> Stage {
        Stage {
            background: look.background.to_string(),
            image: look.image.as_deref().map(shown).unwrap_or_default(),
            text: look.text.to_string(),
            font: look.font.clone().unwrap_or_default(),
            max_size: look.max_size.to_string(),
            min_size: look.min_size.to_string(),
            dim: look.dim.to_string(),
        }
    }

    /// The look these fields say, or why they don't read.
    fn look(&self, path: &dyn Fn(&str) -> PathBuf) -> Result<Look, String> {
        let optional = |text: &str| Some(text.trim().to_string()).filter(|t| !t.is_empty());
        let colour = |text: &str, what: &str| -> Result<Rgb, String> {
            text.parse()
                .map_err(|_| format!("The {what} “{}” isn't a colour like #1a2b3c.", text.trim()))
        };
        let size = |text: &str, what: &str| -> Result<u16, String> {
            text.trim()
                .parse()
                .ok()
                .filter(|size| (1..=400).contains(size))
                .ok_or_else(|| format!("The {what} “{}” isn't a size from 1 to 400.", text.trim()))
        };
        Ok(Look {
            background: colour(&self.background, "background")?,
            image: optional(&self.image).map(|i| path(&i)),
            text: colour(&self.text, "text colour")?,
            font: optional(&self.font),
            max_size: size(&self.max_size, "largest size")?,
            min_size: size(&self.min_size, "smallest size")?,
            dim: self
                .dim
                .trim()
                .parse()
                .ok()
                .filter(|dim| *dim <= 100)
                .ok_or_else(|| {
                    format!(
                        "The dimming “{}” isn't a percentage from 0 to 100.",
                        self.dim.trim()
                    )
                })?,
        })
    }
}

/// A field that suggestions fill: a path, or the stage's font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Folder(usize),
    /// A folder of the open plan's own.
    PlanFolder(usize),
    Bible(usize),
    Colors,
    StageImage,
    Font,
}

impl Field {
    /// What a path field takes; `None` for the font.
    fn want(self) -> Option<Want> {
        Some(match self {
            Field::Folder(_) | Field::PlanFolder(_) => Want::Folders,
            Field::Bible(_) => Want::Files(&["json", "tsv"]),
            Field::Colors => Want::Files(&["json"]),
            Field::StageImage => Want::Files(&["png", "jpg", "jpeg", "webp"]),
            Field::Font => return None,
        })
    }

    /// The field's widget id, to put the cursor at its end after a choice.
    pub fn id(self) -> String {
        match self {
            Field::Folder(i) => format!("settings-folder-{i}"),
            Field::PlanFolder(i) => format!("settings-plan-folder-{i}"),
            Field::Bible(i) => format!("settings-bible-{i}"),
            Field::Colors => "settings-colors".into(),
            Field::StageImage => "settings-stage-image".into(),
            Field::Font => "settings-font".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Folder {
    label: String,
    path: String,
    /// Its bang as typed, `!` or not.
    bang: String,
}

#[derive(Debug, Clone, PartialEq)]
struct Bible {
    name: String,
    lang: Lang,
    path: String,
}

/// One change to the draft.
#[derive(Debug, Clone)]
pub enum Edit {
    FolderLabel(usize, String),
    FolderBang(usize, String),
    FolderPath(usize, String),
    AddFolder,
    RemoveFolder(usize),
    BibleName(usize, String),
    BibleLang(usize, Lang),
    BiblePath(usize, String),
    AddBible,
    RemoveBible(usize),
    BraveKey(String),
    Colors(String),
    /// The path of the open plan's own folder N, a new one, or one less.
    PlanFolderPath(usize, String),
    AddPlanFolder,
    RemovePlanFolder(usize),
    /// The open plan searches the Bible of this name, or stops.
    PlanBible(String),
    /// The key of action N of `keys::ACTIONS`.
    Key(usize, String),
    /// Shows a part of the settings.
    Category(Category),
    /// Waits for action N's new key to be pressed, or stops waiting.
    Record(usize),
    /// Stops waiting for a key.
    StopRecording,
    /// Action N goes back to its usual key.
    ResetKey(usize),
    Background(String),
    StageImage(String),
    /// Put the stage on this screen (`None`: wherever the compositor puts it).
    Output(Option<String>),
    /// Give the open plan its own look, starting from the look for all.
    OwnLook,
    /// The open plan goes back to the look for all plans.
    SharedLook,
    TextColor(String),
    Font(String),
    MaxSize(String),
    MinSize(String),
    Dim(String),
    /// A suggestion was chosen for a path field.
    Choose(Field, String),
    /// Tab: the first suggestion for the field being typed in.
    ChooseFirst,
}

impl Draft {
    /// The fields for `config`, with paths under `home` as `~/…`.
    pub fn of(config: &Config, home: Option<&Path>) -> Draft {
        let shown = |path: &Path| match home {
            Some(home) => config::tilde(path, home).display().to_string(),
            None => path.display().to_string(),
        };
        Draft {
            folders: config
                .folders
                .iter()
                .map(|f| Folder {
                    label: f.label.clone().unwrap_or_default(),
                    path: shown(&f.path),
                    bang: f.bang.as_ref().map(|b| format!("!{b}")).unwrap_or_default(),
                })
                .collect(),
            bibles: config
                .bibles
                .iter()
                .map(|b| Bible {
                    name: b.name.clone().unwrap_or_default(),
                    lang: b.lang,
                    path: shown(&b.path),
                })
                .collect(),
            brave_key: config.brave_api_key.clone().unwrap_or_default(),
            colors: config.colors.as_deref().map(shown).unwrap_or_default(),
            output: config.stage_output.clone(),
            outputs: Err(String::new()),
            stage: Stage::of(&config.stage, &shown),
            plan: None,
            scope: Scope::AllPlans,
            category: Category::Stage,
            recording: None,
            fonts: Vec::new(),
            notice: None,
            home: home.map(Path::to_path_buf),
            suggesting: None,
            saved: String::new(),
            leaving: false,
            // Keys the config gets wrong show as the usual ones.
            keys: {
                let map = Keymap::with(&config.keys).unwrap_or_else(|_| Keymap::defaults());
                (0..keys::ACTIONS.len())
                    .map(|at| map.bind(at).written())
                    .collect()
            },
        }
    }

    /// The action that has the same key as action `at`, if one does: what
    /// Save would refuse. A key that doesn't read clashes with nothing.
    pub fn key_clash(&self, at: usize) -> Option<&'static keys::Action> {
        let read = |key: &String| key.parse::<keys::Bind>().ok();
        let mine = read(self.keys.get(at)?)?;
        self.keys
            .iter()
            .enumerate()
            .find(|(other, key)| *other != at && read(key) == Some(mine))
            .map(|(other, _)| &keys::ACTIONS[other])
    }

    /// Everything Save would keep, as one text to compare.
    fn held(&self) -> String {
        format!(
            "{:?}",
            (
                &self.folders,
                &self.bibles,
                &self.brave_key,
                &self.colors,
                &self.output,
                &self.stage,
                &self.plan,
                &self.keys
            )
        )
    }

    /// The fields are as saved (or as opened) now.
    pub fn mark_saved(&mut self) {
        self.saved = self.held();
        self.leaving = false;
    }

    /// Whether closing now would lose a change.
    pub fn is_dirty(&self) -> bool {
        self.saved != self.held()
    }

    /// The draft, with these font families to suggest.
    pub fn with_fonts(self, fonts: Vec<String>) -> Draft {
        Draft { fonts, ..self }
    }

    /// Applies `edit`. Returns the path field it filled in, if any, so the
    /// view can put the cursor at its end.
    pub fn edit(&mut self, edit: Edit) -> Option<Field> {
        self.notice = None;
        self.leaving = false;
        let typed = match &edit {
            Edit::FolderPath(i, _) => Some(Field::Folder(*i)),
            Edit::PlanFolderPath(i, _) => Some(Field::PlanFolder(*i)),
            Edit::BiblePath(i, _) => Some(Field::Bible(*i)),
            Edit::Colors(_) => Some(Field::Colors),
            Edit::StageImage(_) => Some(Field::StageImage),
            Edit::Font(_) => Some(Field::Font),
            _ => None,
        };
        let chosen = match edit {
            Edit::Choose(field, path) => Some((field, path)),
            Edit::ChooseFirst => self
                .suggesting
                .as_ref()
                .and_then(|(field, found)| Some((*field, found.first()?.path.clone()))),
            edit => {
                self.change(edit);
                None
            }
        };
        if let Some((field, path)) = &chosen {
            self.set_path(*field, path.clone());
        }
        // Keep suggesting while a path is typed or a folder chosen; a
        // chosen file, or any other edit, ends it.
        let field = chosen
            .as_ref()
            .filter(|(_, path)| path.ends_with('/'))
            .map(|(field, _)| *field)
            .or(typed);
        self.suggesting = field.map(|field| (field, self.complete(field)));
        chosen.map(|(field, _)| field)
    }

    /// How many folder rows there are.
    pub fn folder_rows(&self) -> usize {
        self.folders.len()
    }

    /// What `field` could become: what's on disk, or the fonts whose name
    /// has what's typed in it.
    fn complete(&self, field: Field) -> Vec<Suggestion> {
        match field.want() {
            Some(want) => paths::complete(self.path(field), self.home.as_deref(), want),
            None => {
                let typed = fold(self.path(field).trim());
                self.fonts
                    .iter()
                    .filter(|name| fold(name).contains(&typed))
                    .take(paths::MAX)
                    .map(|name| Suggestion {
                        path: name.clone(),
                        folder: false,
                    })
                    .collect()
            }
        }
    }

    fn path(&self, field: Field) -> &str {
        match field {
            Field::Folder(i) => self.folders.get(i).map_or("", |f| f.path.as_str()),
            Field::PlanFolder(i) => self.plan_folder_rows().get(i).map_or("", String::as_str),
            Field::Bible(i) => self.bibles.get(i).map_or("", |b| b.path.as_str()),
            Field::Colors => &self.colors,
            Field::StageImage => &self.stage().image,
            Field::Font => &self.stage().font,
        }
    }

    fn set_path(&mut self, field: Field, path: String) {
        match field {
            Field::Folder(i) => self.folder(i, |f| f.path = path),
            Field::PlanFolder(i) => self.plan_folder(i, path),
            Field::Bible(i) => self.bible(i, |b| b.path = path),
            Field::Colors => self.colors = path,
            Field::StageImage => self.stage_mut().image = path,
            Field::Font => self.stage_mut().font = path,
        }
    }

    fn plan_folder(&mut self, i: usize, path: String) {
        if let Some(folder) = self.plan.as_mut().and_then(|plan| plan.folders.get_mut(i)) {
            *folder = path;
        }
    }

    /// The suggestions under `field`, while it's the one being typed in.
    fn suggestions(&self, field: Field) -> &[Suggestion] {
        match &self.suggesting {
            Some((typed, found)) if *typed == field => found,
            _ => &[],
        }
    }

    fn change(&mut self, edit: Edit) {
        match edit {
            Edit::FolderLabel(i, text) => self.folder(i, |f| f.label = text),
            Edit::FolderBang(i, text) => self.folder(i, |f| f.bang = text),
            Edit::FolderPath(i, text) => self.folder(i, |f| f.path = text),
            Edit::AddFolder => self.folders.push(Folder::default()),
            Edit::PlanFolderPath(i, text) => self.plan_folder(i, text),
            Edit::AddPlanFolder => {
                if let Some(plan) = &mut self.plan {
                    plan.folders.push(String::new());
                }
            }
            Edit::RemovePlanFolder(i) => {
                if let Some(plan) = &mut self.plan
                    && i < plan.folders.len()
                {
                    plan.folders.remove(i);
                }
            }
            Edit::PlanBible(name) => {
                if let Some(plan) = &mut self.plan {
                    match plan
                        .bibles
                        .iter()
                        .position(|b| b.eq_ignore_ascii_case(&name))
                    {
                        Some(at) => {
                            plan.bibles.remove(at);
                        }
                        None => plan.bibles.push(name),
                    }
                }
            }
            Edit::RemoveFolder(i) => {
                if i < self.folders.len() {
                    self.folders.remove(i);
                }
            }
            Edit::BibleName(i, text) => self.bible(i, |b| b.name = text),
            Edit::BibleLang(i, lang) => self.bible(i, |b| b.lang = lang),
            Edit::BiblePath(i, text) => self.bible(i, |b| b.path = text),
            Edit::AddBible => self.bibles.push(Bible {
                name: String::new(),
                lang: Lang::Es,
                path: String::new(),
            }),
            Edit::RemoveBible(i) => {
                if i < self.bibles.len() {
                    self.bibles.remove(i);
                }
            }
            Edit::BraveKey(text) => self.brave_key = text,
            Edit::Colors(text) => self.colors = text,
            Edit::Key(at, text) => {
                if let Some(key) = self.keys.get_mut(at) {
                    *key = text;
                }
                self.recording = None;
            }
            // The plan's part edits the plan's own look; the rest, the
            // look for all plans.
            Edit::Category(category) => {
                self.category = category;
                self.recording = None;
                self.scope = match category {
                    Category::Plan => Scope::ThisPlan,
                    _ => Scope::AllPlans,
                };
            }
            Edit::Record(at) => {
                self.recording = (self.recording != Some(at)).then_some(at);
            }
            Edit::StopRecording => self.recording = None,
            Edit::ResetKey(at) => {
                if let (Some(key), Some(action)) = (self.keys.get_mut(at), keys::ACTIONS.get(at)) {
                    *key = action.key.to_string();
                }
                self.recording = None;
            }
            Edit::Output(output) => {
                self.category = Category::Output;
                self.output = output;
            }
            Edit::OwnLook => {
                let all = self.stage.clone();
                if let Some(plan) = &mut self.plan {
                    plan.own.get_or_insert(all);
                }
            }
            Edit::SharedLook => {
                if let Some(plan) = &mut self.plan {
                    plan.own = None;
                }
            }
            Edit::Background(text) => self.stage_mut().background = text,
            Edit::StageImage(text) => self.stage_mut().image = text,
            Edit::TextColor(text) => self.stage_mut().text = text,
            Edit::Font(text) => self.stage_mut().font = text,
            Edit::MaxSize(text) => self.stage_mut().max_size = text,
            Edit::MinSize(text) => self.stage_mut().min_size = text,
            Edit::Dim(text) => self.stage_mut().dim = text,
            Edit::Choose(..) | Edit::ChooseFirst => {}
        }
    }

    fn folder(&mut self, i: usize, change: impl FnOnce(&mut Folder)) {
        if let Some(folder) = self.folders.get_mut(i) {
            change(folder);
        }
    }

    fn bible(&mut self, i: usize, change: impl FnOnce(&mut Bible)) {
        if let Some(bible) = self.bibles.get_mut(i) {
            change(bible);
        }
    }

    /// The config the fields describe, read the way the file is (`~`, paths
    /// relative to `dir`). Rows left empty are dropped; a named row without
    /// a path is an error that says which.
    pub fn config(&self, dir: &Path, home: Option<&Path>) -> Result<Config, String> {
        let path = |text: &str| config::resolve(Path::new(text.trim()), dir, home);
        let optional = |text: &str| Some(text.trim().to_string()).filter(|t| !t.is_empty());
        let mut folders = Vec::new();
        for folder in &self.folders {
            match (optional(&folder.label), optional(&folder.path)) {
                (_, Some(p)) => folders.push(FolderEntry {
                    path: path(&p),
                    label: optional(&folder.label),
                    bang: optional(&folder.bang)
                        .map(|bang| config::folder_bang(&bang))
                        .transpose()
                        .map_err(|err| format!("The folder {p}: {err}"))?,
                }),
                (Some(label), None) => return Err(format!("The folder “{label}” needs a path.")),
                (None, None) => {}
            }
        }
        let mut bibles = Vec::new();
        for bible in &self.bibles {
            match (optional(&bible.name), optional(&bible.path)) {
                (_, Some(p)) => bibles.push(BibleEntry {
                    lang: bible.lang,
                    path: path(&p),
                    name: optional(&bible.name),
                }),
                (Some(name), None) => return Err(format!("The Bible “{name}” needs a file.")),
                (None, None) => {}
            }
        }
        let stage = self.stage.look(&|text: &str| path(text))?;
        // An emptied field goes back to the usual key.
        let typed: BTreeMap<String, String> = keys::ACTIONS
            .iter()
            .zip(&self.keys)
            .filter(|(_, key)| !key.trim().is_empty())
            .map(|(action, key)| (action.name.to_string(), key.clone()))
            .collect();
        let keys = Keymap::with(&typed)?.changed();
        Ok(Config {
            keys,
            brave_api_key: optional(&self.brave_key),
            colors: optional(&self.colors).map(|c| path(&c)),
            stage_output: self.output.clone(),
            folders,
            bibles,
            stage,
        })
    }

    /// Also edits `title`'s look: `look` is its own, `None` when it uses
    /// the look for all plans.
    pub fn with_plan(mut self, plan: &Plan) -> Draft {
        let (title, look, folders) = (&plan.title, plan.look.as_ref(), &plan.folders);
        let home = self.home.clone();
        let shown = |path: &Path| match &home {
            Some(home) => config::tilde(path, home).display().to_string(),
            None => path.display().to_string(),
        };
        self.plan = Some(PlanLook {
            title: title.to_string(),
            own: look.map(|look| Stage::of(look, &shown)),
            folders: folders.iter().map(|f| f.display().to_string()).collect(),
            bibles: plan.bibles.clone(),
        });
        self
    }

    /// The open plan's own folders as typed, without the empty rows;
    /// `None` when no plan is being edited. Kept as typed (`~/…`), so the
    /// plan reads the same on another machine.
    pub fn plan_folders(&self) -> Option<Vec<PathBuf>> {
        let plan = self.plan.as_ref()?;
        Some(
            plan.folders
                .iter()
                .map(|folder| folder.trim())
                .filter(|folder| !folder.is_empty())
                .map(PathBuf::from)
                .collect(),
        )
    }

    /// Whether the folder typed in `field` isn't on this computer: not
    /// while it's still being typed and has somewhere to go, and not for
    /// an empty row.
    pub fn missing_folder(
        &self,
        field: Field,
        typed: &str,
        dir: &Path,
        home: Option<&Path>,
    ) -> bool {
        let typed = typed.trim();
        let going = self
            .suggesting
            .as_ref()
            .is_some_and(|(at, to)| *at == field && !to.is_empty());
        !typed.is_empty() && !going && !config::resolve(Path::new(typed), dir, home).is_dir()
    }

    /// The folders of the draft that aren't on this computer, as typed:
    /// the app's and the open plan's.
    pub fn missing_folders(&self, dir: &Path, home: Option<&Path>) -> Vec<String> {
        let app = self.folders.iter().map(|folder| folder.path.as_str());
        let plan = self.plan_folder_rows().iter().map(String::as_str);
        app.chain(plan)
            .map(str::trim)
            .filter(|typed| !typed.is_empty())
            .filter(|typed| !config::resolve(Path::new(typed), dir, home).is_dir())
            .map(str::to_string)
            .collect()
    }

    /// The Bibles the open plan searches, by name; empty for all of them.
    /// `None` when no plan is being edited. A name that's no longer one
    /// of the Bibles here is kept: the plan may go back to where it is.
    pub fn plan_bibles(&self) -> Option<Vec<String>> {
        self.plan.as_ref().map(|plan| plan.bibles.clone())
    }

    /// The Bibles there are to pick from, by name, and whether the open
    /// plan picked each.
    fn bible_choices(&self) -> Vec<(String, bool)> {
        let picked = self.plan.as_ref().map_or(&[][..], |p| p.bibles.as_slice());
        self.bibles
            .iter()
            .filter(|bible| !bible.path.trim().is_empty())
            .map(|bible| {
                let name = library::name_of_bible(Some(&bible.name), Path::new(bible.path.trim()));
                let on = picked.iter().any(|b| b.eq_ignore_ascii_case(&name));
                (name, on)
            })
            .collect()
    }

    /// The field of the plan's last folder row, to type in once it's added.
    pub fn last_plan_folder(&self) -> Option<Field> {
        self.plan_folder_rows()
            .len()
            .checked_sub(1)
            .map(Field::PlanFolder)
    }

    fn plan_folder_rows(&self) -> &[String] {
        self.plan
            .as_ref()
            .map_or(&[], |plan| plan.folders.as_slice())
    }

    /// Lists the connected screens, for the output row.
    pub fn with_outputs(mut self, outputs: Result<Vec<Output>, String>) -> Draft {
        self.outputs = outputs;
        self
    }

    /// The open plan's title, when the settings edit its look too.
    pub fn plan_title(&self) -> Option<&str> {
        self.plan.as_ref().map(|plan| plan.title.as_str())
    }

    /// Whether the open plan has a look of its own.
    pub fn plan_has_own(&self) -> bool {
        self.plan.as_ref().is_some_and(|plan| plan.own.is_some())
    }

    /// The look fields being edited: the plan's own in its scope, else the
    /// look for all plans.
    fn stage(&self) -> &Stage {
        match (self.scope, &self.plan) {
            (Scope::ThisPlan, Some(PlanLook { own: Some(own), .. })) => own,
            _ => &self.stage,
        }
    }

    fn stage_mut(&mut self) -> &mut Stage {
        match (self.scope, &mut self.plan) {
            (Scope::ThisPlan, Some(PlanLook { own: Some(own), .. })) => own,
            _ => &mut self.stage,
        }
    }

    /// Whether the fields shown edit a look: not in a plan's scope while
    /// it uses the look for all plans.
    pub fn editing_a_look(&self) -> bool {
        self.scope == Scope::AllPlans || self.plan_has_own()
    }

    /// The open plan's look as the fields say: `Some(None)` for the look for
    /// all plans; `None` when no plan is being edited.
    pub fn plan_look(
        &self,
        dir: &Path,
        home: Option<&Path>,
    ) -> Result<Option<Option<Look>>, String> {
        let Some(plan) = &self.plan else {
            return Ok(None);
        };
        let path = |text: &str| config::resolve(Path::new(text.trim()), dir, home);
        match &plan.own {
            None => Ok(Some(None)),
            Some(own) => own
                .look(&path)
                .map(|look| Some(Some(look)))
                .map_err(|err| format!("This plan's look: {err}")),
        }
    }
}

impl App {
    pub(crate) fn settings_view<'a>(&'a self, draft: &'a Draft) -> Element<'a, Message> {
        let p = self.palette;
        let file = self.config_status.path();
        let shown_file = match &self.home {
            Some(home) => config::tilde(file, home).display().to_string(),
            None => file.display().to_string(),
        };
        let mut header = row![
            look::icon(Icon::Settings, 20.0, p.primary),
            text("Settings").size(HEADING).font(BOLD).color(p.primary),
            Space::new().width(Fill),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .height(match (&self.stage.program, self.stage.blackout) {
            (Some(_), false) => LIVE_HEIGHT,
            _ => 36.0,
        });
        // The settings take the stage pane's room, so what the audience
        // sees is kept in sight here: its state, its name, a small picture.
        match (&self.stage.program, self.stage.blackout) {
            (_, true) => header = header.push(chip(p, "blackout", Tone::Danger)),
            (Some(scene), false) => {
                header = header
                    .push(chip(p, "● live", Tone::Live))
                    .push(
                        iced::widget::container(
                            crate::dashboard::single_line(scene.resource.title.clone())
                                .size(META)
                                .color(p.on_surface),
                        )
                        .max_width(220)
                        .clip(true),
                    )
                    .push(
                        iced::widget::container(self.monitor(
                            scene,
                            self.media.content(scene.id),
                            None,
                        ))
                        .width(LIVE_HEIGHT * 16.0 / 9.0)
                        .height(LIVE_HEIGHT),
                    );
            }
            (None, false) => {}
        }
        header = header.push(
            text(format!("{} closes", keys::label(Op::ClearSearch)))
                .size(META)
                .color(p.on_surface_variant),
        );

        let mut folders = column![section(
            p,
            "Media folders",
            "The folders Lantern indexes, with those inside them. A bang like !h searches only that folder."
        )]
        .spacing(6);
        // A folder that isn't there says so under its row, once it's
        // typed: saved as it is, it would just never find anything.
        let config_dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
        let home = self.home.clone();
        let not_there = move |draft: &Draft, field: Field, typed: &str| {
            draft
                .missing_folder(field, typed, &config_dir, home.as_deref())
                .then(|| -> Element<'a, Message> {
                    text("Not found on this computer. It's skipped until it's there.")
                        .size(META)
                        .color(p.error)
                        .into()
                })
        };
        for (i, folder) in draft.folders.iter().enumerate() {
            // The path has a line to itself: beside the label and the bang
            // it showed about twenty characters of a folder's path.
            folders = folders.push(
                row![
                    field(p, "Label", &folder.label, move |t| Edit::FolderLabel(i, t)).width(Fill),
                    field(p, "!bang", &folder.bang, move |t| Edit::FolderBang(i, t))
                        .width(BANG_FIELD),
                    remove(p, Edit::RemoveFolder(i)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
            folders = folders.push(
                field(p, "e.g. ~/Music", &folder.path, move |t| {
                    Edit::FolderPath(i, t)
                })
                .id(Field::Folder(i).id())
                .width(Fill),
            );
            folders = folders.push(suggestions(p, draft, Field::Folder(i)));
            folders = folders.push(not_there(draft, Field::Folder(i), &folder.path));
        }
        folders = folders.push(add(p, "Add folder", Edit::AddFolder));

        let mut bibles = column![section(
            p,
            "Bibles",
            "One .json or .tsv file per translation."
        )]
        .spacing(6);
        for (i, bible) in draft.bibles.iter().enumerate() {
            let lang = |label: &'static str, value: Lang| {
                let tone = if bible.lang == value {
                    Tone::Selected
                } else {
                    Tone::Plain
                };
                pill(
                    p,
                    label,
                    tone,
                    PILL,
                    Some(settings(Edit::BibleLang(i, value))),
                )
            };
            bibles = bibles.push(
                row![
                    field(p, "Name", &bible.name, move |t| Edit::BibleName(i, t)).width(Fill),
                    lang("es", Lang::Es),
                    lang("en", Lang::En),
                    remove(p, Edit::RemoveBible(i)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
            bibles = bibles.push(
                field(p, "e.g. ~/bibles/rv1909.tsv", &bible.path, move |t| {
                    Edit::BiblePath(i, t)
                })
                .id(Field::Bible(i).id())
                .width(Fill),
            );
            bibles = bibles.push(suggestions(p, draft, Field::Bible(i)));
        }
        bibles = bibles.push(add(p, "Add Bible", Edit::AddBible));

        let images = column![
            section(
                p,
                "Image search",
                "!gi needs a Brave Search API key (brave.com/search/api). It's billed to you."
            ),
            field(p, "Brave Search API key", &draft.brave_key, Edit::BraveKey).secure(true),
        ]
        .spacing(6);
        let colors = column![
            section(
                p,
                "Colours",
                "A Matugen colours file, to follow your desktop's theme. Empty uses Lantern's own."
            ),
            field(
                p,
                "~/.config/amane/colors.json",
                &draft.colors,
                Edit::Colors
            )
            .id(Field::Colors.id()),
            suggestions(p, draft, Field::Colors),
        ]
        .spacing(6);

        let small = |placeholder, value, edit: fn(String) -> Edit| {
            field(p, placeholder, value, edit).width(SMALL_FIELD)
        };
        let note = |name| text(name).size(META).color(p.on_surface_variant);
        let label = |name| text(name).size(BODY).font(MEDIUM).width(LABEL_FIELD);
        let line =
            |items: iced::widget::Row<'a, Message>| items.spacing(8).align_y(Alignment::Center);
        let mut stage = column![section(
            p,
            if draft.scope == Scope::ThisPlan { "Its look" } else { "Stage" },
            "What's behind the content, and how text is set: as large as fits between the two sizes."
        )]
        .spacing(6);
        // The plan's part: the look for all plans, or one of its own.
        {
            if draft.scope == Scope::ThisPlan {
                let (about, action, tone, edit) = if draft.plan_has_own() {
                    (
                        "This plan has its own look, saved with it.",
                        "Use the look for all plans",
                        Tone::Plain,
                        Edit::SharedLook,
                    )
                } else {
                    (
                        "This plan uses the look for all plans.",
                        "Give it its own look",
                        Tone::Selected,
                        Edit::OwnLook,
                    )
                };
                stage = stage.push(row![
                    Space::new().width(LABEL_FIELD),
                    column![
                        note(about),
                        pill(p, action, tone, look::PILL_SMALL, Some(settings(edit))),
                    ]
                    .spacing(6),
                ]);
            }
        }
        if draft.editing_a_look() {
            stage = stage.extend([
                line(row![
                    label("Background"),
                    small("#000000", &draft.stage().background, Edit::Background),
                    swatch(&draft.stage().background),
                ])
                .into(),
                line(row![
                    label("Text colour"),
                    small("#ffffff", &draft.stage().text, Edit::TextColor),
                    swatch(&draft.stage().text),
                ])
                .into(),
                line(row![
                    label("Text size"),
                    small("40", &draft.stage().min_size, Edit::MinSize),
                    note("to"),
                    small("96", &draft.stage().max_size, Edit::MaxSize),
                    note("pt"),
                ])
                .into(),
                line(row![
                    label("Image"),
                    field(
                        p,
                        "none: ~/Pictures/cross.jpg",
                        &draft.stage().image,
                        Edit::StageImage
                    )
                    .id(Field::StageImage.id())
                    .width(Fill),
                ])
                .into(),
                suggestions(p, draft, Field::StageImage),
                line(row![
                    label("Dim image"),
                    small("60", &draft.stage().dim, Edit::Dim),
                    note("% under text, so it reads"),
                ])
                .into(),
                line(row![
                    label("Font"),
                    field(
                        p,
                        "the system's own: type to find one",
                        &draft.stage().font,
                        Edit::Font
                    )
                    .id(Field::Font.id())
                    .width(Fill),
                ])
                .into(),
                suggestions(p, draft, Field::Font),
            ]);
        }
        stage =
            stage.push(row![Space::new().width(LABEL_FIELD), self.stage_sample(draft)].spacing(8));

        let mut keys = column![section(
            p,
            "Keys",
            "Click a key, then press the new one. Esc gives up; ↺ puts the usual key back."
        )]
        .spacing(4);
        for (at, (action, key)) in keys::ACTIONS.iter().zip(&draft.keys).enumerate() {
            let recording = draft.recording == Some(at);
            let shown = if recording {
                "Press a key…".to_string()
            } else {
                keys::shown(key)
            };
            // Two actions on one key show as soon as it's pressed, on both
            // rows: Save would only refuse it later.
            let clash = draft.key_clash(at).filter(|_| !recording);
            let tone = match (recording, clash) {
                (true, _) => Tone::Selected,
                (false, Some(_)) => Tone::Danger,
                (false, None) => Tone::Plain,
            };
            // Only a changed key has something to put back.
            let reset: Element<'a, Message> = if key != action.key {
                iced::widget::tooltip(
                    button(text("↺").size(BODY).color(p.primary))
                        .padding([4, 8])
                        .on_press(settings(Edit::ResetKey(at)))
                        .style(look::row(p, Tone::Plain)),
                    iced::widget::container(
                        text(format!("Back to {}", keys::shown(action.key))).size(META),
                    )
                    .padding([4, 8])
                    .style(look::tip(p)),
                    iced::widget::tooltip::Position::Top,
                )
                .into()
            } else {
                Space::new().width(28).into()
            };
            keys = keys.push(line(row![
                text(action.about).size(BODY).width(Fill),
                reset,
                pill(
                    p,
                    &shown,
                    tone,
                    look::PILL_SMALL,
                    Some(settings(Edit::Record(at)))
                )
                .width(SMALL_FIELD + 40.0),
            ]));
            if let Some(other) = clash {
                keys = keys.push(
                    text(format!(
                        "{} is also the key for “{}”. One of them needs another.",
                        keys::shown(key),
                        other.about
                    ))
                    .size(META)
                    .color(p.error)
                    .width(Fill)
                    .align_x(iced::alignment::Horizontal::Right),
                );
            }
        }

        // Opened for a plan's look, the stage comes first.
        let output = column![
            section(
                p,
                "Output",
                "The screen the stage goes on. Picking one moves it there now; Save keeps it."
            ),
            output_row(p, draft),
        ]
        .spacing(6);
        // Media only this plan needs: indexed while it's open.
        let mut plan_folders = column![section(
            p,
            "Its folders",
            "Media only this plan needs. Searched while the plan is open, and not after."
        )]
        .spacing(6);
        for (i, folder) in draft.plan_folder_rows().iter().enumerate() {
            plan_folders = plan_folders.push(
                row![
                    field(p, "e.g. ~/Church/Campaña", folder, move |t| {
                        Edit::PlanFolderPath(i, t)
                    })
                    .id(Field::PlanFolder(i).id())
                    .width(Fill),
                    remove(p, Edit::RemovePlanFolder(i)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
            plan_folders = plan_folders.push(suggestions(p, draft, Field::PlanFolder(i)));
            plan_folders = plan_folders.push(not_there(draft, Field::PlanFolder(i), folder));
        }
        plan_folders = plan_folders.push(add(p, "Add folder", Edit::AddPlanFolder));

        // Which Bibles it searches: all of them until one is picked.
        let choices = draft.bible_choices();
        let all = !choices.iter().any(|(_, on)| *on);
        let mut plan_bibles = column![section(
            p,
            "Its Bibles",
            "The Bibles this plan searches. All of them, until you pick."
        )]
        .spacing(6);
        // What the plan asks for by name, whether or not it's here.
        let picked = draft.plan_bibles().unwrap_or_default();
        let here: Vec<&String> = choices.iter().map(|(name, _)| name).collect();
        let elsewhere: Vec<&String> = picked
            .iter()
            .filter(|name| !here.iter().any(|h| h.eq_ignore_ascii_case(name)))
            .collect();
        let says = |words: String| text(words).size(META).color(p.on_surface_variant);
        if choices.is_empty() {
            plan_bibles = plan_bibles.push(says("No Bibles yet: add one under Library.".into()));
        } else if choices.len() == 1 && picked.is_empty() {
            // One Bible is no choice: a toggle here would change nothing.
            plan_bibles = plan_bibles.push(says(format!(
                "{} is the only Bible here, so this plan searches it. \
                 With another added under Library, you can pick.",
                choices[0].0
            )));
        } else {
            let ticked: Vec<String> = choices
                .iter()
                .filter(|(_, on)| *on)
                .map(|(name, _)| name.clone())
                .collect();
            let pills = choices.into_iter().map(|(name, on)| {
                let label = if on {
                    format!("✓ {name}")
                } else {
                    name.clone()
                };
                let tone = if on { Tone::Selected } else { Tone::Plain };
                let edit = settings(Edit::PlanBible(name));
                pill(p, &label, tone, look::PILL_SMALL, Some(edit)).into()
            });
            // In words too: a tick alone doesn't say whether the rest
            // are still searched.
            let state = if all {
                "Searching all of them.".to_string()
            } else {
                format!("Searching only {}.", ticked.join(" and "))
            };
            plan_bibles = plan_bibles.push(
                row(pills)
                    .push(says(state))
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .wrap(),
            );
        }
        if !elsewhere.is_empty() {
            let names: Vec<&str> = elsewhere.iter().map(|name| name.as_str()).collect();
            plan_bibles = plan_bibles.push(says(format!(
                "This plan also asks for {}, which isn't on this computer{}.",
                names.join(" and "),
                if all {
                    ": until it is, it searches them all"
                } else {
                    ""
                }
            )));
        }

        // One part at a time, picked from the list beside it.
        let part = match draft.category {
            Category::Plan => column![self.plan_section(), plan_folders, plan_bibles, stage],
            Category::Stage => column![stage],
            Category::Output => column![output],
            Category::Library => column![folders, bibles, images],
            Category::Appearance => column![colors],
            Category::Keys => column![keys],
        };
        let mut parts = vec![
            Category::Stage,
            Category::Output,
            Category::Library,
            Category::Appearance,
            Category::Keys,
        ];
        if draft.plan_title().is_some() {
            parts.insert(0, Category::Plan);
        }
        let list = column(parts.into_iter().map(|category| {
            let on = draft.category == category;
            button(
                text(category.name())
                    .size(BODY)
                    .font(if on { BOLD } else { MEDIUM })
                    .color(if on { p.primary } else { p.on_surface }),
            )
            .width(Fill)
            .padding([8, 12])
            .on_press(settings(Edit::Category(category)))
            .style(look::row(p, if on { Tone::Selected } else { Tone::Plain }))
            .into()
        }))
        .spacing(2)
        .width(CATEGORIES);
        let body = row![
            list,
            scrolling(iced::widget::container(part.spacing(24)).max_width(PART_WIDTH)).id(SCROLL),
        ]
        .spacing(24)
        .height(Fill);
        // Where it's all kept, for whoever edits the file by hand.
        let mut footer = row![chip(p, &shown_file, Tone::Plain)]
            .spacing(8)
            .align_y(Alignment::Center);
        if let Some(notice) = &draft.notice {
            let (message, color) = match notice {
                Ok(done) if self.library.0.indexing() => {
                    (format!("{done} Reindexing…"), p.on_surface_variant)
                }
                Ok(done) => (done.clone(), p.on_surface_variant),
                Err(why) => (why.clone(), p.error),
            };
            footer = footer.push(
                text(message)
                    .size(META)
                    .color(color)
                    .width(Fill)
                    .align_x(iced::alignment::Horizontal::Right),
            );
        } else {
            footer = footer.push(Space::new().width(Fill));
        }
        // Unsaved changes say so, and closing then says it discards them.
        let dirty = draft.is_dirty();
        if dirty && !draft.leaving {
            footer = footer.push(chip(p, "unsaved changes", Tone::Live));
        }
        footer = footer
            .push(pill(
                p,
                &keys::with_key(
                    match (dirty, draft.leaving) {
                        (true, true) => "Discard them",
                        (true, false) => "Discard",
                        (false, _) => "Close",
                    },
                    Op::ClearSearch,
                ),
                if dirty && draft.leaving {
                    Tone::Danger
                } else {
                    Tone::Plain
                },
                PILL,
                Some(Message::Key {
                    op: Op::ClearSearch,
                    only_if_empty: false,
                }),
            ))
            .push(pill(
                p,
                &keys::with_key("Save", Op::Save),
                Tone::Selected,
                PILL,
                Some(Message::SaveSettings),
            ));
        column![header, body, footer]
            .spacing(16)
            .height(Fill)
            .into()
    }
}

impl App {
    /// The open plan itself: its name, whether it's a template, a copy of
    /// it, and deleting it. These act on the plan at once; its Save keeps
    /// the name and the template mark.
    fn plan_section<'a>(&self) -> Element<'a, Message> {
        let p = self.palette;
        let plan = self.engine.0.plan();
        let label = |name| text(name).size(BODY).font(MEDIUM).width(LABEL_FIELD);
        let note = |words: &'static str| text(words).size(META).color(p.on_surface_variant);
        let name = text_input("Plan name", &plan.title)
            .on_input(Message::RenamePlan)
            .size(BODY)
            .font(MEDIUM)
            .padding([8, 10])
            .style(look::input(p));
        let template = pill(
            p,
            if plan.template {
                "✓ A template"
            } else {
                "Make it a template"
            },
            if plan.template {
                Tone::Selected
            } else {
                Tone::Plain
            },
            look::PILL_SMALL,
            Some(Message::PlanTemplate(!plan.template)),
        );
        let delete = pill(
            p,
            if self.deleting_plan {
                "Delete it for good?"
            } else {
                "Delete"
            },
            Tone::Danger,
            look::PILL_SMALL,
            Some(Message::DeletePlan),
        );
        // As typed while it doesn't read as a time yet.
        let starts = match (&self.starts_typed, plan.starts) {
            (Some(typed), _) => typed.clone(),
            (None, Some(starts)) => starts.to_string(),
            (None, None) => String::new(),
        };
        let starts = text_input("e.g. 10:30", &starts)
            .on_input(Message::PlanStarts)
            .size(BODY)
            .padding([8, 10])
            .width(SMALL_FIELD)
            .style(look::input(p));
        column![
            section(p, "This plan", "Its name, and what can be done with it."),
            row![label("Name"), name]
                .spacing(8)
                .align_y(Alignment::Center),
            row![
                label("Starts at"),
                starts,
                note("When the service begins: sections say when they're due.")
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![
                label("Template"),
                template,
                note("A template starts new plans with its order of service.")
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![
                label("Copy"),
                pill(
                    p,
                    "Duplicate",
                    Tone::Plain,
                    look::PILL_SMALL,
                    Some(Message::DuplicatePlan)
                ),
                note("Saves this plan, then opens a copy of it.")
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![
                label("Take along"),
                pill(
                    p,
                    if self.copying {
                        "Exporting…"
                    } else {
                        "Export"
                    },
                    Tone::Plain,
                    look::PILL_SMALL,
                    (!self.copying).then_some(Message::ExportPlan)
                ),
                note("One file in Documents: the plan and its media.")
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![
                label("Delete"),
                delete,
                note("Removes the plan and its file.")
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(6)
        .into()
    }

    /// A small stage with the look as typed, showing a verse, so changes
    /// show before they're saved. Fields that don't read yet keep the
    /// saved look's value.
    fn stage_sample<'a>(&self, draft: &Draft) -> Element<'a, Message> {
        let saved = &self.config.stage;
        let look = Look {
            background: draft.stage().background.parse().unwrap_or(saved.background),
            text: draft.stage().text.parse().unwrap_or(saved.text),
            image: Some(draft.stage().image.trim())
                .filter(|i| !i.is_empty())
                .and_then(|i| {
                    let home = self.home.as_deref();
                    let dir = self.config_status.path().parent()?;
                    Some(config::resolve(Path::new(i), dir, home))
                }),
            font: Some(draft.stage().font.trim().to_string()).filter(|f| !f.is_empty()),
            max_size: draft
                .stage
                .max_size
                .trim()
                .parse()
                .unwrap_or(saved.max_size),
            min_size: draft
                .stage
                .min_size
                .trim()
                .parse()
                .unwrap_or(saved.min_size),
            dim: draft
                .stage()
                .dim
                .trim()
                .parse()
                .ok()
                .filter(|dim| *dim <= 100)
                .unwrap_or(saved.dim),
        };
        let sample = crate::backdrop(
            &look,
            fit::fit_text(
                "Porque de tal manera amó Dios al mundo,\nque ha dado á su Hijo unigénito",
                &look,
            ),
            crate::Over::Text,
        );
        iced::widget::container(sample)
            .width(SAMPLE_WIDTH)
            .height(SAMPLE_WIDTH * 9.0 / 16.0)
            .into()
    }
}

/// A dot of the colour typed, or nothing while it doesn't read.
fn swatch<'a>(typed: &str) -> Element<'a, Message> {
    let Ok(rgb) = typed.parse::<Rgb>() else {
        return Space::new().width(18).into();
    };
    let colour = fit::color(rgb);
    iced::widget::container(Space::new())
        .width(18)
        .height(18)
        .style(move |_| iced::widget::container::Style {
            background: Some(colour.into()),
            border: iced::Border::default()
                .rounded(9)
                .width(1)
                .color(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.3)),
            ..Default::default()
        })
        .into()
}

fn settings(edit: Edit) -> Message {
    Message::Settings(edit)
}

/// A section's heading and one line on what it's for.
fn section<'a>(p: Palette, title: &'a str, about: &'a str) -> Element<'a, Message> {
    column![
        text(title).size(HEADING).font(BOLD).color(p.primary),
        text(about).size(META).color(p.on_surface_variant),
    ]
    .spacing(2)
    .into()
}

fn field<'a>(
    p: Palette,
    placeholder: &str,
    value: &str,
    edit: impl Fn(String) -> Edit + 'a,
) -> text_input::TextInput<'a, Message> {
    text_input(placeholder, value)
        .on_input(move |text| settings(edit(text)))
        .size(BODY)
        .font(MEDIUM)
        .padding([8, 10])
        .style(look::input(p))
}

/// What a path field could become, under it while it's typed in: click one,
/// or Tab for the first.
fn suggestions<'a>(p: Palette, draft: &Draft, field: Field) -> Element<'a, Message> {
    let found = draft.suggestions(field);
    if found.is_empty() {
        return Space::new().into();
    }
    let mut list = column![].spacing(2);
    for (n, suggestion) in found.iter().enumerate() {
        let icon = if suggestion.folder {
            Icon::Folder
        } else {
            Icon::Document
        };
        let mut line = row![
            look::icon(icon, 14.0, p.on_surface_variant),
            text(suggestion.path.clone()).size(BODY).width(Fill),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if n == 0 {
            line = line.push(text("Tab").size(META).color(p.on_surface_variant));
        }
        list = list.push(
            button(line)
                .width(Fill)
                .padding([6, 10])
                .on_press(settings(Edit::Choose(field, suggestion.path.clone())))
                .style(look::row(p, Tone::Plain)),
        );
    }
    // Under its field: a folder's or Bible's path starts at the edge, the
    // stage's image and font after their label.
    let indent = match field {
        Field::Folder(_) | Field::PlanFolder(_) | Field::Bible(_) => 0.0,
        _ => LABEL_FIELD + 6.0,
    };
    iced::widget::container(list)
        .padding(iced::Padding::default().left(indent))
        .into()
}

/// The screens to put the stage on: Automatic, then each connected one,
/// and a line when the one asked for isn't connected.
fn output_row<'a>(p: Palette, draft: &Draft) -> Element<'a, Message> {
    let note = |line: String| text(line).size(META).color(p.on_surface_variant);
    let choices: Element<'a, Message> = match &draft.outputs {
        Err(why) => note(why.clone()).into(),
        Ok(outputs) => {
            let choice = |label: String, output: Option<String>| {
                let tone = if draft.output == output {
                    Tone::Selected
                } else {
                    Tone::Plain
                };
                pill(
                    p,
                    &label,
                    tone,
                    look::PILL_SMALL,
                    Some(settings(Edit::Output(output))),
                )
            };
            let mut list = column![choice("Automatic".into(), None)].spacing(6);
            for output in outputs {
                list = list.push(choice(output.label(), Some(output.name.clone())));
            }
            if let Some(wanted) = &draft.output
                && !outputs.iter().any(|o| &o.name == wanted)
            {
                list = list.push(note(format!(
                    "{wanted} isn't connected: the stage moves there when it is."
                )));
            }
            list.into()
        }
    };
    choices
}

fn remove(p: Palette, edit: Edit) -> Element<'static, Message> {
    button(text("✕").size(META).color(p.on_surface_variant))
        .padding([6, 8])
        .on_press(settings(edit))
        .style(look::row(p, Tone::Plain))
        .into()
}

fn add<'a>(p: Palette, label: &str, edit: Edit) -> Element<'a, Message> {
    row![
        wide(pill(
            p,
            label,
            Tone::Plain,
            look::PILL_SMALL,
            Some(settings(edit))
        ))
        .width(160)
    ]
    .into()
}

/// What a save says once written: where.
pub fn saved(path: &Path, home: Option<&Path>) -> String {
    let shown = match home {
        Some(home) => config::tilde(path, home),
        None => path.to_path_buf(),
    };
    format!("Saved to {}.", shown.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> Option<&'static Path> {
        Some(Path::new("/home/ana"))
    }

    fn sample() -> Config {
        Config::parse(
            r#"
            colors = "~/.config/amane/colors.json"
            [[folders]]
            path = "~/Music"
            label = "Music"
            [[bibles]]
            lang = "es"
            path = "/srv/rv1909.tsv"
            "#,
            Path::new("/home/ana/.config/lantern"),
            home(),
        )
        .unwrap()
    }

    #[test]
    fn a_plan_gets_its_own_look_without_touching_the_one_for_all() {
        let dir = Path::new("/home/ana/.config/lantern");
        let mut draft = Draft::of(&sample(), home()).with_plan(&Plan::new("Santa Cena"));
        assert_eq!(
            draft.plan_look(dir, home()).unwrap(),
            Some(None),
            "uses the shared look"
        );
        draft.edit(Edit::Category(Category::Plan));
        assert!(
            !draft.editing_a_look(),
            "nothing to edit until it has its own"
        );

        draft.edit(Edit::OwnLook);
        draft.edit(Edit::Background("#301010".into()));
        let own = draft.plan_look(dir, home()).unwrap().unwrap().unwrap();
        assert_eq!(own.background, Rgb([0x30, 0x10, 0x10]));
        assert_eq!(
            draft.config(dir, home()).unwrap().stage,
            sample().stage,
            "all plans unchanged"
        );

        draft.edit(Edit::Category(Category::Stage));
        draft.edit(Edit::Background("#000080".into()));
        assert_eq!(
            draft
                .plan_look(dir, home())
                .unwrap()
                .unwrap()
                .unwrap()
                .background,
            Rgb([0x30, 0x10, 0x10]),
            "the plan keeps its own"
        );

        draft.edit(Edit::Category(Category::Plan));
        draft.edit(Edit::Background("not a colour".into()));
        assert!(
            draft
                .plan_look(dir, home())
                .unwrap_err()
                .starts_with("This plan's look:")
        );
        draft.edit(Edit::SharedLook);
        assert_eq!(draft.plan_look(dir, home()).unwrap(), Some(None));
        assert_eq!(
            Draft::of(&sample(), home()).plan_look(dir, home()).unwrap(),
            None,
            "no plan"
        );
    }

    #[test]
    fn two_actions_on_one_key_clash_at_once() {
        let mut draft = Draft::of(&sample(), home());
        let at = |name: &str| keys::ACTIONS.iter().position(|a| a.name == name).unwrap();
        let (blackout, save) = (at("blackout"), at("save"));
        assert!((0..keys::ACTIONS.len()).all(|at| draft.key_clash(at).is_none()));
        draft.edit(Edit::Key(blackout, "ctrl+s".into()));
        assert_eq!(draft.key_clash(blackout).map(|a| a.name), Some("save"));
        assert_eq!(draft.key_clash(save).map(|a| a.name), Some("blackout"));
        draft.edit(Edit::ResetKey(blackout));
        assert_eq!(draft.key_clash(save).map(|a| a.name), None);
    }

    #[test]
    fn a_draft_knows_when_closing_would_lose_a_change() {
        let mut draft = Draft::of(&sample(), home()).with_plan(&Plan::new("Santa Cena"));
        draft.mark_saved();
        assert!(!draft.is_dirty());
        // Looking around changes nothing.
        draft.edit(Edit::Category(Category::Keys));
        draft.edit(Edit::Record(0));
        draft.edit(Edit::StopRecording);
        assert!(!draft.is_dirty());
        draft.edit(Edit::Background("#102030".into()));
        assert!(draft.is_dirty());
        draft.leaving = true;
        draft.edit(Edit::AddPlanFolder);
        assert!(!draft.leaving, "an edit takes the question back");
        draft.mark_saved();
        assert!(!draft.is_dirty());
    }

    #[test]
    fn a_folder_that_isnt_there_is_named() {
        let here = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(here.path().join("Media")).unwrap();
        let mut plan = Plan::new("Campaña");
        plan.folders = vec!["Media".into(), "nowhere".into()];
        let mut draft = Draft::of(&Config::default(), None).with_plan(&plan);
        draft.edit(Edit::AddFolder);
        draft.edit(Edit::FolderPath(0, "gone".into()));
        draft.edit(Edit::AddFolder);
        // Relative to the config's folder; an empty row isn't missing.
        assert_eq!(
            draft.missing_folders(here.path(), None),
            ["gone", "nowhere"]
        );
        let field = Field::PlanFolder(1);
        assert!(draft.missing_folder(field, "nowhere", here.path(), None));
        assert!(!draft.missing_folder(Field::PlanFolder(0), "Media", here.path(), None));
        assert!(!draft.missing_folder(field, "  ", here.path(), None));
    }

    #[test]
    fn a_plan_picks_its_bibles_by_name() {
        let mut plan = Plan::new("Bilingual");
        plan.bibles = vec!["NVI".into()];
        let mut draft = Draft::of(&sample(), home()).with_plan(&plan);
        let names: Vec<String> = draft.bible_choices().into_iter().map(|(n, _)| n).collect();
        assert!(!names.is_empty(), "the sample has a Bible");
        assert!(draft.bible_choices().iter().all(|(_, on)| !on));
        draft.edit(Edit::PlanBible(names[0].clone()));
        assert!(draft.bible_choices()[0].1);
        // One the computer doesn't have stays in the plan.
        assert_eq!(
            draft.plan_bibles(),
            Some(vec!["NVI".to_string(), names[0].clone()])
        );
        draft.edit(Edit::PlanBible(names[0].to_lowercase()));
        assert_eq!(draft.plan_bibles(), Some(vec!["NVI".to_string()]));
    }

    #[test]
    fn the_fields_show_home_as_a_tilde_and_read_back_the_same() {
        let draft = Draft::of(&sample(), home());
        assert_eq!(draft.folders[0].path, "~/Music");
        assert_eq!(draft.colors, "~/.config/amane/colors.json");
        let dir = Path::new("/home/ana/.config/lantern");
        assert_eq!(draft.config(dir, home()).unwrap(), sample());
    }

    #[test]
    fn empty_rows_are_dropped_and_a_named_row_needs_a_path() {
        let dir = Path::new("/home/ana/.config/lantern");
        let mut draft = Draft::of(&sample(), home());
        draft.edit(Edit::AddFolder);
        draft.edit(Edit::BraveKey("  ".into()));
        let config = draft.config(dir, home()).unwrap();
        assert_eq!(config.folders.len(), 1, "the empty row is dropped");
        assert_eq!(config.brave_api_key, None);

        draft.edit(Edit::FolderLabel(1, "Videos".into()));
        let err = draft.config(dir, home()).unwrap_err();
        assert_eq!(err, "The folder “Videos” needs a path.");

        draft.edit(Edit::FolderPath(1, "~/Himnos".into()));
        draft.edit(Edit::FolderBang(1, "!H".into()));
        let config = draft.config(dir, home()).unwrap();
        assert_eq!(config.folders[1].bang.as_deref(), Some("h"));
        assert_eq!(Draft::of(&config, home()).folders[1].bang, "!h");
        draft.edit(Edit::FolderBang(1, "b".into()));
        let err = draft.config(dir, home()).unwrap_err();
        assert_eq!(
            err,
            "The folder ~/Himnos: !b is taken by Lantern's own search: pick another."
        );
        draft.edit(Edit::FolderBang(1, "him nos".into()));
        assert!(
            draft.config(dir, home()).is_err(),
            "a space can't be in a bang"
        );

        draft.edit(Edit::RemoveFolder(1));
        draft.edit(Edit::AddBible);
        draft.edit(Edit::BiblePath(1, "~/kjv.json".into()));
        draft.edit(Edit::BibleLang(1, Lang::En));
        let config = draft.config(dir, home()).unwrap();
        assert_eq!(config.bibles[1].path, Path::new("/home/ana/kjv.json"));
        assert_eq!(config.bibles[1].lang, Lang::En);
    }

    #[test]
    fn typing_a_path_suggests_and_choosing_a_folder_keeps_going() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("Music/Himnario")).unwrap();
        let mut draft = Draft::of(&Config::default(), Some(home.path()));
        draft.edit(Edit::AddFolder);
        draft.edit(Edit::FolderPath(0, "~/Mu".into()));
        assert_eq!(draft.suggestions(Field::Folder(0))[0].path, "~/Music/");
        assert!(
            draft.suggestions(Field::Colors).is_empty(),
            "only under the field typed in"
        );

        assert_eq!(draft.edit(Edit::ChooseFirst), Some(Field::Folder(0)));
        assert_eq!(draft.folders[0].path, "~/Music/");
        assert_eq!(
            draft.suggestions(Field::Folder(0))[0].path,
            "~/Music/Himnario/"
        );

        draft.edit(Edit::FolderLabel(0, "Himnos".into()));
        assert!(
            draft.suggestions(Field::Folder(0)).is_empty(),
            "another edit ends it"
        );
    }

    #[test]
    fn the_font_field_suggests_installed_families_by_any_part_of_the_name() {
        let fonts = vec!["DejaVu Sans".into(), "DejaVu Serif".into(), "Inter".into()];
        let mut draft = Draft::of(&Config::default(), home()).with_fonts(fonts);
        draft.edit(Edit::Font("serif".into()));
        let found: Vec<&str> = draft
            .suggestions(Field::Font)
            .iter()
            .map(|s| s.path.as_str())
            .collect();
        assert_eq!(found, ["DejaVu Serif"]);
        assert_eq!(draft.edit(Edit::ChooseFirst), Some(Field::Font));
        assert_eq!(draft.stage().font, "DejaVu Serif");
        assert!(
            draft.suggestions(Field::Font).is_empty(),
            "a chosen font ends it"
        );
    }

    #[test]
    fn the_stage_fields_read_back_and_say_what_doesnt_read() {
        let dir = Path::new("/home/ana/.config/lantern");
        let mut draft = Draft::of(&Config::default(), home());
        assert!(draft.config(dir, home()).unwrap().stage.is_default());

        draft.edit(Edit::Background("#123".into()));
        draft.edit(Edit::StageImage("~/Pictures/cross.jpg".into()));
        draft.edit(Edit::MaxSize("120".into()));
        let stage = draft.config(dir, home()).unwrap().stage;
        assert_eq!(stage.background, Rgb([0x11, 0x22, 0x33]));
        assert_eq!(
            stage.image.as_deref(),
            Some(Path::new("/home/ana/Pictures/cross.jpg"))
        );
        assert_eq!(stage.max_size, 120);

        draft.edit(Edit::TextColor("white".into()));
        let err = draft.config(dir, home()).unwrap_err();
        assert_eq!(err, "The text colour “white” isn't a colour like #1a2b3c.");
        draft.edit(Edit::TextColor("#fff".into()));
        draft.edit(Edit::MinSize("0".into()));
        let err = draft.config(dir, home()).unwrap_err();
        assert_eq!(err, "The smallest size “0” isn't a size from 1 to 400.");
        draft.edit(Edit::MinSize("40".into()));
        draft.edit(Edit::Dim("150".into()));
        let err = draft.config(dir, home()).unwrap_err();
        assert_eq!(err, "The dimming “150” isn't a percentage from 0 to 100.");
        draft.edit(Edit::Dim("35".into()));
        assert_eq!(draft.config(dir, home()).unwrap().stage.dim, 35);
    }
}
