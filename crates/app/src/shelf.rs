//! The library, beside the plan while planning: Lantern's own sources and
//! steps, then the media folders as a tree, to build a plan by browsing as
//! well as by searching. A source opens the search on it. A click looks
//! at a file, on this screen only; a double click or Enter adds it to the
//! plan, after the step that is selected.
//!
//! The keyboard walks the tree too, once the arrows are given to it: ↑ ↓
//! move through the rows in sight, → and ← open and close folders.
//!
//! Each folder lays out its files as a list, as a grid of even tiles, or
//! as tiles at their pictures' own shapes ([`Layout`]); the arrows walk
//! the tiles in their order as they walk rows.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::widget::{
    Space, button, column, container, mouse_area, operation, responsive, row, scrollable, sensor,
    stack, text, text_input, tooltip,
};
use iced::{Alignment, Color, Element, Fill, Length, Padding, Task};
use lantern_engine::{Action, Bang, Location, Query, Resource, ResourceId};

use crate::dashboard::{OWN_SOURCES, bang_about};
use crate::gallery;
use crate::look::{self, BODY, BOLD, Icon, MEDIUM, META, Tone};
use crate::palette::Palette;
use crate::plan_edit::NewStep;
use crate::plan_pane::{Known, Thumb, kind_word};
use crate::track::{self, Key};
use crate::widgets::{caption, hinted, scrolling, single_line};
use crate::{App, Clicked, Message};

/// The most rows the tree draws at once. A folder of six hundred hymns
/// is all there, a filter away; past this many, the filter is the way.
const MOST: usize = 1000;

/// How wide a file's picture is: a little under a plan step's.
const PICTURE: f32 = 52.0;
/// How far out of sight a file's row may be when its picture is asked for.
const SIGHT: f32 = 400.0;

/// The id of the library's filter box, to focus it.
pub const FILTER: &str = "library-filter";

/// The id of the tree's rows, to scroll them.
pub const ROWS: &str = "library-rows";

/// The folders between a file and its library folder, as a path:
/// `Hymns/Kids`. Empty for a file right in the folder.
fn within(file: &Resource) -> String {
    file.tags.get(1..).unwrap_or_default().join("/")
}

/// What of the library the pointer is over: one of its files, or one of
/// the steps it always has.
#[derive(Debug, Clone, PartialEq)]
pub enum Grip {
    File(ResourceId),
    New(NewStep),
}

/// What is held: a file of the library, or a step it always has. A
/// new step is not a file, so it is not made to look like one.
pub enum Held {
    File(Arc<Resource>),
    New(NewStep),
}

/// Something of the library held by the pointer, on its way into the plan.
pub struct Carried {
    pub what: Held,
    /// The other files picked with it, carried after it in their order.
    pub more: Vec<Arc<Resource>>,
    /// Where it was picked up, and where the pointer is now.
    from: Option<iced::Point>,
    pub at: iced::Point,
    /// It has moved far enough to be a drag and not a click.
    pub moving: bool,
    /// Where it would land, among the plan's rows: kept from the last
    /// row the pointer was over, so that the place opening for it, which
    /// moves that row from under the pointer, does not take it away.
    pub landing: Option<usize>,
}

impl Carried {
    pub fn new(what: Held) -> Carried {
        Carried {
            what,
            more: Vec::new(),
            from: None,
            at: iced::Point::ORIGIN,
            moving: false,
            landing: None,
        }
    }

    /// The file held, when it is one.
    pub fn file(&self) -> Option<&Arc<Resource>> {
        match &self.what {
            Held::File(file) => Some(file),
            Held::New(_) => None,
        }
    }

    /// What it goes by while it is held.
    pub fn title(&self) -> String {
        match &self.what {
            Held::File(file) => file.title.clone(),
            Held::New(new) => new.step().0.to_string(),
        }
    }

    pub fn icon(&self) -> Icon {
        match &self.what {
            Held::File(file) => Icon::of(&file.kind),
            Held::New(new) => new_icon(*new),
        }
    }

    /// The steps letting it go puts in the plan, in their order.
    pub fn steps(&self) -> Vec<(String, Action)> {
        match &self.what {
            Held::File(file) => std::iter::once(file)
                .chain(&self.more)
                .map(|file| {
                    (
                        file.title.clone(),
                        Action::Show {
                            resource: (**file).clone(),
                        },
                    )
                })
                .collect(),
            Held::New(new) => {
                let (label, action) = new.step();
                vec![(label.to_string(), action)]
            }
        }
    }

    /// The pointer moved to `at`. A few pixels of travel make it a drag.
    pub fn to(&mut self, at: iced::Point) {
        let from = *self.from.get_or_insert(at);
        self.at = at;
        if from.distance(at) > 6.0 {
            self.moving = true;
        }
    }
}

/// The icon of a step the library always has.
fn new_icon(new: NewStep) -> Icon {
    match new {
        NewStep::LiveSlot => Icon::Screen,
        NewStep::BlackScreen => Icon::ScreenOff,
        NewStep::Section => Icon::Plus,
    }
}

/// A row of the library's own group, before the folders.
pub(crate) struct Own {
    icon: Icon,
    name: &'static str,
    /// What it says quietly on the right: which Bibles.
    detail: String,
    /// What its tip says.
    about: String,
    does: Does,
}

/// What a press on a row of the library's own group does.
pub(crate) enum Does {
    /// The search opens on the bang, as typing it would.
    Search(String),
    /// The step is added after the selected one. Pressed and pulled, it
    /// is carried into the plan like a file.
    Add(NewStep),
}

impl Own {
    fn press(&self) -> Message {
        match &self.does {
            Does::Search(bang) => Message::Bang(bang.clone()),
            Does::Add(new) => Message::AddStep(*new),
        }
    }
}

/// Lantern's own sources among `bangs`, the searcher's, in their order.
/// The folders' own source is left out: the folders have a group of
/// their own.
fn own_sources(bangs: &[Bang]) -> Vec<Own> {
    OWN_SOURCES
        .iter()
        .filter(|(bang, ..)| *bang != "f")
        .filter_map(|&(bang, icon, name)| {
            let found = bangs.iter().find(|found| found.bang == bang)?;
            // A source named for what it is says nothing more; a Bible is
            // named for its translation, worth saying.
            let detail = found
                .sources
                .iter()
                .filter(|source| !source.starts_with(name))
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            Some(Own {
                icon,
                name,
                detail,
                about: bang_about(found),
                does: Does::Search(bang.to_string()),
            })
        })
        .collect()
}

/// A row of the library tree, as it is drawn.
pub(crate) enum ShelfRow {
    Folder {
        /// Its name, and its path from the library folder: what opens it.
        label: String,
        key: String,
        /// How many files are under it.
        count: usize,
        depth: usize,
        open: bool,
    },
    File {
        file: Arc<Resource>,
        depth: usize,
        /// Where in its folder it is, while filtering.
        note: String,
    },
}

impl ShelfRow {
    fn depth(&self) -> usize {
        match self {
            ShelfRow::Folder { depth, .. } | ShelfRow::File { depth, .. } => *depth,
        }
    }

    fn is(&self, at: &ShelfAt) -> bool {
        match (self, at) {
            (ShelfRow::Folder { key, .. }, ShelfAt::Folder(folder)) => key == folder,
            (ShelfRow::File { file, .. }, ShelfAt::File(id)) => &file.id == id,
            _ => false,
        }
    }

    fn name(&self) -> &str {
        match self {
            ShelfRow::Folder { label, .. } => label,
            ShelfRow::File { file, .. } => &file.title,
        }
    }

    /// What a press on it does: a folder opens or closes, a file is
    /// looked at.
    fn press(&self) -> Message {
        match self {
            ShelfRow::Folder { key, .. } => Message::ShelfFold(key.clone()),
            ShelfRow::File { file, .. } => Message::Click(Clicked::Shelf(file.id.clone())),
        }
    }
}

/// Where the keyboard is in the library.
enum ShelfAt {
    Folder(String),
    File(lantern_engine::ResourceId),
}

/// Where the arrows, Home and End take a list's selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Go {
    Next,
    Prev,
    First,
    Last,
}

impl Go {
    /// The row to be on, of `len`, from row `at`. The arrows go round.
    pub(crate) fn from(self, at: Option<usize>, len: usize) -> Option<usize> {
        match (at, self) {
            _ if len == 0 => None,
            (_, Go::First) | (None, Go::Next) => Some(0),
            (_, Go::Last) | (None, Go::Prev) => Some(len - 1),
            (Some(at), Go::Next) => Some((at + 1) % len),
            (Some(at), Go::Prev) => Some((at + len - 1) % len),
        }
    }
}

/// One folder of the library and the files in it.
pub struct Folder {
    /// Its label, as in Settings.
    pub label: String,
    pub files: Vec<Arc<Resource>>,
}

/// How a folder of the library lays out its files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Layout {
    /// A row each: its picture, its name and a line of what it is.
    #[default]
    List,
    /// Even 16:9 tiles, two to a row, each with its name and its line
    /// under it.
    Grid,
    /// Each tile at its picture's own shape, the rows packed edge to edge.
    Shapes,
}

impl Layout {
    const ALL: [Layout; 3] = [Layout::List, Layout::Grid, Layout::Shapes];

    /// What it is called in a script and in the file it is kept in.
    fn word(self) -> &'static str {
        match self {
            Layout::List => "list",
            Layout::Grid => "grid",
            Layout::Shapes => "shapes",
        }
    }

    /// The layout called `word`.
    pub fn named(word: &str) -> Option<Layout> {
        let word = word.trim().to_lowercase();
        Layout::ALL.into_iter().find(|layout| layout.word() == word)
    }

    fn icon(self) -> Icon {
        match self {
            Layout::List => Icon::ViewList,
            Layout::Grid => Icon::ViewGrid,
            Layout::Shapes => Icon::ViewShapes,
        }
    }

    /// What its switch's tip says, laying out `folder`.
    fn about(self, folder: &str) -> String {
        match self {
            Layout::List => format!("List: {folder}, a row for each file"),
            Layout::Grid => format!("Grid: {folder} in even tiles, two to a row"),
            Layout::Shapes => format!("Shapes: {folder}, each picture at its own shape"),
        }
    }
}

/// The layout each folder of the library was given, by its label. A
/// folder given none is a list. They are kept for the next start in a
/// file beside the saved plans, a line each: the label, a tab, and the
/// layout's word.
pub struct Layouts {
    file: PathBuf,
    given: BTreeMap<String, Layout>,
}

impl Layouts {
    /// The layouts kept in the data folder `data`. A line that can't be
    /// read is left out.
    pub fn read(data: &Path) -> Layouts {
        let file = data.join("library-layouts");
        let given = std::fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .filter_map(|(folder, word)| Some((folder.to_string(), Layout::named(word)?)))
            .collect();
        Layouts { file, given }
    }

    /// How the folder labelled `folder` lays out its files.
    pub fn of(&self, folder: &str) -> Layout {
        self.given.get(folder).copied().unwrap_or_default()
    }

    /// Lays out `folder` as `layout`, and keeps that for the next start.
    pub fn set(&mut self, folder: &str, layout: Layout) -> std::io::Result<()> {
        match layout {
            Layout::List => self.given.remove(folder),
            layout => self.given.insert(folder.to_string(), layout),
        };
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let lines: String = self
            .given
            .iter()
            .map(|(folder, layout)| format!("{folder}\t{}\n", layout.word()))
            .collect();
        std::fs::write(&self.file, lines)
    }
}

/// The folder of the library, among `rows`, that the layout switch is
/// for: the open one holding the row the keyboard or the pointer is
/// `at`, else the last one open. None while every folder is closed.
fn here(rows: &[ShelfRow], at: Option<&ShelfAt>) -> Option<String> {
    let open = |row: &&ShelfRow| {
        matches!(
            row,
            ShelfRow::Folder {
                depth: 0,
                open: true,
                ..
            }
        )
    };
    let at = at.and_then(|at| rows.iter().position(|row| row.is(at)));
    let holding = at
        .and_then(|at| rows[..=at].iter().rfind(|row| row.depth() == 0))
        .filter(open);
    match holding.or_else(|| rows.iter().rfind(open))? {
        ShelfRow::Folder { key, .. } => Some(key.clone()),
        ShelfRow::File { .. } => None,
    }
}

/// What a file of the library says of itself, the same in every layout.
struct Facts {
    /// It is picked, alone or with others.
    picked: bool,
    /// What is known of it, once it has come near sight.
    known: Option<Known>,
    /// The plan shows it, whole or a page of it.
    in_plan: bool,
    /// The quiet line under its name, and its colour.
    under: String,
    colour: Color,
}

impl Facts {
    /// Its picture's facts: what it is of, its size and its state. Only
    /// its kind's icon until it comes near sight.
    fn thumb<'a>(&self, file: &'a Resource) -> Thumb<'a> {
        match self.known {
            Some(known) => Thumb::of(file, known),
            None => Thumb::kind(Icon::of(&file.kind)),
        }
    }
}

/// A file of the library drawn as a tile: its place among the tree's
/// rows, and what it says of itself.
struct Tile {
    at: usize,
    file: Arc<Resource>,
    facts: Facts,
}

/// How far apart the Grid's tiles are, and the room round a tile's
/// picture and words, which its fill shows in when picked or hovered.
const GRID_GAP: f32 = 2.0;
const TILE_ROOM: f32 = 4.0;
/// How far apart the pictures of the Shapes are.
const SHAPES_GAP: f32 = 4.0;

impl App {
    /// Reads the folders again when the index has changed, while they
    /// are in sight.
    pub(crate) fn sync_shelf(&mut self) {
        if self.mode != crate::mode::Mode::Plan {
            return;
        }
        let changes = self.library.0.changes();
        if self.shelf_read == Some(changes) {
            return;
        }
        self.shelf_read = Some(changes);
        // Everything on disk, by the folder it is in: a file's first tag
        // is its folder's label.
        let mut folders: Vec<Folder> = Vec::new();
        let all = Query::new("").with_limit(usize::MAX);
        for hit in self.engine.0.search_index(&all) {
            let resource = hit.resource;
            if !matches!(resource.location, Location::File(_)) {
                continue;
            }
            let Some(label) = resource.tags.first() else {
                continue;
            };
            match folders.iter_mut().find(|folder| &folder.label == label) {
                Some(folder) => folder.files.push(resource),
                None => folders.push(Folder {
                    label: label.clone(),
                    files: vec![resource],
                }),
            }
        }
        folders.sort_by_key(|folder| folder.label.to_lowercase());
        self.shelf = folders;
    }

    /// The file of the library with this id.
    pub(crate) fn shelf_file(&self, id: &lantern_engine::ResourceId) -> Option<Arc<Resource>> {
        self.shelf
            .iter()
            .flat_map(|folder| &folder.files)
            .find(|file| &file.id == id)
            .cloned()
    }

    /// The rows of the library tree that are in sight, top to bottom,
    /// and how many files were left out for being too many. With
    /// something typed in the filter, the files whose name or folder has
    /// it, flat under their folder.
    pub(crate) fn shelf_rows(&self) -> (Vec<ShelfRow>, usize) {
        let filter = lantern_engine::fold(self.shelf_filter.trim());
        let mut rows = Vec::new();
        let mut drawn = 0;
        let mut left_out = 0;
        let mut file = |rows: &mut Vec<ShelfRow>, file: &Arc<Resource>, depth, note| {
            if drawn >= MOST {
                left_out += 1;
                return;
            }
            drawn += 1;
            rows.push(ShelfRow::File {
                file: file.clone(),
                depth,
                note,
            });
        };
        for folder in &self.shelf {
            // Filtering: the files that have it, flat under their folder,
            // each saying where in it they are.
            if !filter.is_empty() {
                let found: Vec<&Arc<Resource>> = folder
                    .files
                    .iter()
                    .filter(|file| {
                        lantern_engine::fold(&file.title).contains(&filter)
                            || lantern_engine::fold(&within(file)).contains(&filter)
                    })
                    .collect();
                if found.is_empty() {
                    continue;
                }
                rows.push(ShelfRow::Folder {
                    label: folder.label.clone(),
                    key: folder.label.clone(),
                    count: found.len(),
                    depth: 0,
                    open: true,
                });
                for found in found {
                    file(&mut rows, found, 1, within(found));
                }
                continue;
            }
            let open = self.shelf_open.contains(&folder.label);
            rows.push(ShelfRow::Folder {
                label: folder.label.clone(),
                key: folder.label.clone(),
                count: folder.files.len(),
                depth: 0,
                open,
            });
            if !open {
                continue;
            }
            // The folders inside it, each with what is under it, in the
            // order their paths sort; then the files right in it.
            let mut paths: Vec<String> = folder
                .files
                .iter()
                .map(|file| within(file))
                .filter(|path| !path.is_empty())
                .collect();
            paths.sort();
            paths.dedup();
            // Every folder on the way to a file is a folder too.
            let mut folders: Vec<String> = Vec::new();
            for path in &paths {
                let mut at = String::new();
                for part in path.split('/') {
                    if !at.is_empty() {
                        at.push('/');
                    }
                    at.push_str(part);
                    if !folders.contains(&at) {
                        folders.push(at.clone());
                    }
                }
            }
            folders.sort();
            let files_in = |path: &str| -> Vec<&Arc<Resource>> {
                folder
                    .files
                    .iter()
                    .filter(|file| within(file) == path)
                    .collect()
            };
            for path in &folders {
                // In sight only if every folder above it is open.
                let key = format!("{}/{path}", folder.label);
                let parent_open = path.rsplit_once('/').is_none_or(|(above, _)| {
                    let mut at = String::new();
                    above.split('/').all(|part| {
                        if !at.is_empty() {
                            at.push('/');
                        }
                        at.push_str(part);
                        self.shelf_open.contains(&format!("{}/{at}", folder.label))
                    })
                });
                if !parent_open {
                    continue;
                }
                let depth = path.matches('/').count() + 1;
                let under = folder
                    .files
                    .iter()
                    .filter(|file| {
                        let at = within(file);
                        at == *path || at.starts_with(&format!("{path}/"))
                    })
                    .count();
                let open = self.shelf_open.contains(&key);
                let name = path.rsplit('/').next().unwrap_or(path).to_string();
                rows.push(ShelfRow::Folder {
                    label: name,
                    key,
                    count: under,
                    depth,
                    open,
                });
                if !open {
                    continue;
                }
                for found in files_in(path) {
                    file(&mut rows, found, depth + 1, String::new());
                }
            }
            for found in files_in("") {
                file(&mut rows, found, 1, String::new());
            }
        }
        (rows, left_out)
    }

    /// Whether the arrows are the library's: it was gone to, and it is
    /// in sight.
    pub(crate) fn keys_in_library(&self) -> bool {
        self.in_library && self.mode == crate::mode::Mode::Plan
    }

    /// Where the keyboard is in the library: a folder, or the file that
    /// is picked.
    fn shelf_at(&self) -> Option<ShelfAt> {
        match &self.shelf_folder {
            Some(folder) => Some(ShelfAt::Folder(folder.clone())),
            None => self.shelf_pick.clone().map(ShelfAt::File),
        }
    }

    /// Gives the arrows to the library, or back to the plan. Coming in
    /// with nothing picked, they start on its first row.
    pub(crate) fn switch_pane(&mut self) -> Task<Message> {
        self.in_library = !self.in_library;
        // The library is one lens of the side panel: the arrows can only
        // go to it while it is the one in sight.
        if self.in_library {
            self.lens = crate::side::Lens::Add;
        }
        if self.in_library && self.shelf_at().is_none() {
            return self.shelf_move(Go::First);
        }
        Task::none()
    }

    /// Moves through the library's rows.
    pub(crate) fn shelf_move(&mut self, go: Go) -> Task<Message> {
        let (rows, _) = self.shelf_rows();
        let at = self
            .shelf_at()
            .and_then(|at| rows.iter().position(|row| row.is(&at)));
        match go.from(at, rows.len()) {
            Some(to) => self.shelf_go(&rows, to),
            None => Task::none(),
        }
    }

    /// → and ← in the library, as in any tree: → opens a folder and then
    /// steps into it; ← closes it, and from inside goes up to it.
    pub(crate) fn shelf_side(&mut self, into: bool) -> Task<Message> {
        let (rows, _) = self.shelf_rows();
        let Some(at) = self
            .shelf_at()
            .and_then(|at| rows.iter().position(|row| row.is(&at)))
        else {
            return self.shelf_move(Go::First);
        };
        // Filtered, every folder is open and stays so.
        let folds = self.shelf_filter.trim().is_empty();
        let above = rows[..at]
            .iter()
            .rposition(|row| row.depth() < rows[at].depth());
        match &rows[at] {
            ShelfRow::Folder { key, open, .. } if into != *open && folds => {
                if !self.shelf_open.remove(key) {
                    self.shelf_open.insert(key.clone());
                }
                Task::none()
            }
            ShelfRow::Folder { depth, .. } if into => match rows.get(at + 1) {
                Some(inside) if inside.depth() > *depth => self.shelf_go(&rows, at + 1),
                _ => Task::none(),
            },
            _ if into => Task::none(),
            _ => match above {
                Some(above) => self.shelf_go(&rows, above),
                None => Task::none(),
            },
        }
    }

    /// Files of the library picked together, in the tree's order: a
    /// press with Ctrl adds one or drops it, with Shift takes all from the
    /// one picked to it; with neither, they are let go.
    pub(crate) fn shelf_pick_many(&mut self, id: &ResourceId, ctrl: bool, shift: bool) {
        if !ctrl && !shift {
            self.shelf_many.clear();
            return;
        }
        let (rows, _) = self.shelf_rows();
        let order: Vec<ResourceId> = rows
            .iter()
            .filter_map(|row| match row {
                ShelfRow::File { file, .. } => Some(file.id.clone()),
                ShelfRow::Folder { .. } => None,
            })
            .collect();
        if shift {
            let at = |want: &ResourceId| order.iter().position(|id| id == want);
            if let (Some(from), Some(to)) = (self.shelf_pick.as_ref().and_then(at), at(id)) {
                self.shelf_many = order[from.min(to)..=from.max(to)].to_vec();
            }
        } else {
            if self.shelf_many.is_empty()
                && let Some(first) = self.shelf_pick.clone()
            {
                self.shelf_many.push(first);
            }
            match self.shelf_many.iter().position(|picked| picked == id) {
                Some(at) => {
                    self.shelf_many.remove(at);
                }
                None => self.shelf_many.push(id.clone()),
            }
            self.shelf_many
                .sort_by_key(|picked| order.iter().position(|id| id == picked));
        }
    }

    /// Enter on a folder of the library opens or closes it.
    pub(crate) fn shelf_enter(&mut self) -> bool {
        let Some(folder) = self.shelf_folder.clone() else {
            return false;
        };
        if !self.shelf_open.remove(&folder) {
            self.shelf_open.insert(folder);
        }
        true
    }

    /// Puts the keyboard on row `to`: a file is looked at, as a click
    /// would; a folder only lights up. The tree scrolls to keep it in
    /// sight.
    fn shelf_go(&mut self, rows: &[ShelfRow], to: usize) -> Task<Message> {
        let looked = match &rows[to] {
            ShelfRow::Folder { key, .. } => {
                self.shelf_folder = Some(key.clone());
                self.shelf_pick = None;
                Task::none()
            }
            ShelfRow::File { file, .. } => {
                self.shelf_folder = None;
                self.search.unpick();
                self.shelf_pick = Some(file.id.clone());
                self.engine.0.preview(file.clone());
                self.preload()
            }
        };
        let along = match rows.len() {
            0 | 1 => 0.0,
            len => to as f32 / (len - 1) as f32,
        };
        Task::batch([
            looked,
            operation::snap_to(
                ROWS,
                scrollable::RelativeOffset {
                    x: None,
                    y: Some(along),
                },
            ),
        ])
    }

    /// The library: its own group, Lantern's sources and the steps it
    /// always has, then the folders as a tree, each with the folders
    /// inside it and its files, laid out as the folder they are in is.
    pub(crate) fn shelf_pane(&self) -> Element<'_, Message> {
        let p = self.palette;
        let keys = self.keys_in_library();
        let (tree, left_out) = self.shelf_rows();
        // A small grey name over each group, and space above it. The
        // folders' is lit while the arrows walk them.
        let group = |words: &'static str, top: f32, lit: bool| {
            container(caption(p, words).color(match lit {
                true => p.primary,
                false => p.on_surface_variant,
            }))
            .padding(Padding::default().top(top).left(8.0).bottom(4.0))
        };
        let mut rows = column![group("LANTERN", 2.0, false)].spacing(2);
        for own in self.own_rows() {
            rows = rows.push(self.own_row(own));
        }
        let folders = group("YOUR FOLDERS", 14.0, keys).width(Fill);
        if self.shelf.is_empty() {
            rows = rows.push(folders);
            rows = rows.push(
                container(
                    text("No media folders yet. Add them in Settings › Library.")
                        .size(BODY)
                        .color(p.on_surface_variant),
                )
                .padding([4, 8]),
            );
        } else {
            // Laid over the heading, centred on its words: the heading is
            // no taller for it.
            let switch = container(self.layout_switch(here(&tree, self.shelf_at().as_ref())))
                .align_right(Fill)
                .center_y(Fill)
                .padding(Padding::default().top(14.0 - 4.0).right(8.0));
            rows = rows.push(stack![folders, switch]);
        }
        // A file the plan shows, whole or a page of it, says so.
        let plan = self.engine.0.plan();
        let planned: HashSet<&Path> = plan
            .steps()
            .iter()
            .filter_map(|step| match &step.action {
                Action::Show { resource } => match &resource.location {
                    Location::File(path) => Some(path.as_path()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        // A row of the tree: a folder with how many files are under it,
        // or a file as a plan step's row is, its picture with its size, its
        // name, and one quiet line of what it is.
        let tree_row = |row: &ShelfRow| -> Element<'_, Message> {
            let indent = Padding::default()
                .left(8.0 + 18.0 * row.depth() as f32)
                .right(8.0);
            match row {
                ShelfRow::Folder {
                    label,
                    key,
                    count,
                    open,
                    ..
                } => {
                    let at = keys && self.shelf_folder.as_ref() == Some(key);
                    let fill = match (at, open) {
                        (true, _) => look::row_fill(p, Tone::Selected),
                        (false, true) => look::pill_colours(p, Tone::Plain).0,
                        (false, false) => Color::TRANSPARENT,
                    };
                    button(
                        row![
                            look::icon(Icon::Folder, 16.0, p.on_surface_variant),
                            container(
                                single_line(label.clone())
                                    .size(BODY)
                                    .font(if at { BOLD } else { MEDIUM })
                                    .color(if at { p.primary } else { p.on_surface }),
                            )
                            .width(Fill)
                            .clip(true),
                            text(count.to_string())
                                .size(META)
                                .color(p.on_surface_variant),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    )
                    .width(Fill)
                    .padding(indent.top(8.0).bottom(8.0))
                    .on_press(row.press())
                    .style(look::row_filled(p, fill))
                    .into()
                }
                ShelfRow::File { file, note, .. } => {
                    let facts = self.facts(file, note, &planned);
                    let picture =
                        self.near_sight(file, self.row_picture(&facts.thumb(file), PICTURE));
                    let line = row![
                        picture,
                        column![
                            single_line(file.title.clone())
                                .size(BODY)
                                .font(if facts.picked { BOLD } else { MEDIUM })
                                .color(if facts.picked {
                                    p.primary
                                } else {
                                    p.on_surface
                                }),
                            single_line(facts.under).size(META).color(facts.colour),
                        ]
                        .spacing(1),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center);
                    self.held(
                        file,
                        button(container(line).clip(true))
                            .width(Fill)
                            .padding(indent.top(4.0).bottom(4.0))
                            .on_press(row.press())
                            .style(look::row(p, picked_tone(facts.picked))),
                    )
                }
            }
        };
        let files = tree
            .iter()
            .filter(|row| matches!(row, ShelfRow::File { .. }))
            .count();
        // The rows in their order; the files right in a folder side by
        // side where its library folder is laid out so.
        let mut library = None;
        let mut at = 0;
        while at < tree.len() {
            if let ShelfRow::Folder { depth: 0, key, .. } = &tree[at] {
                library = Some(key.as_str());
            }
            let layout = library.map_or(Layout::List, |folder| self.shelf_layouts.of(folder));
            let run = tree[at..]
                .iter()
                .take_while(|row| matches!(row, ShelfRow::File { .. }))
                .count();
            let tiles = || self.tiles(&tree, at..at + run, &planned);
            let (drawn, next) = match (layout, run) {
                (Layout::List, _) | (_, 0) => (
                    track::seen(Key::Shelf(at), &self.places, tree_row(&tree[at])),
                    at + 1,
                ),
                (Layout::Grid, _) => (self.grid(tiles(), tree[at].depth()), at + run),
                (Layout::Shapes, _) => (self.shapes(tiles(), tree[at].depth()), at + run),
            };
            rows = rows.push(drawn);
            at = next;
        }
        if !self.shelf_filter.trim().is_empty() && files == 0 && !self.shelf.is_empty() {
            rows = rows.push(
                container(
                    text(format!(
                        "Nothing in the library has “{}”.",
                        self.shelf_filter.trim()
                    ))
                    .size(BODY)
                    .color(p.on_surface_variant),
                )
                .padding([4, 8]),
            );
        }
        if left_out > 0 {
            rows = rows.push(
                container(
                    text(format!("and {left_out} more: type above to narrow it"))
                        .size(META)
                        .color(p.on_surface_variant),
                )
                .padding([4, 8]),
            );
        }
        let rows = scrolling(track::seen(Key::ShelfRows, &self.places, rows))
            .id(ROWS)
            .on_scroll(|seen| Message::ShelfScrolled(seen.absolute_offset().y));
        column![
            track::seen(Key::ShelfView, &self.places, rows),
            Space::new().height(4)
        ]
        .spacing(8)
        .width(Fill)
        .height(Fill)
        .into()
    }

    /// The folder of the library that the layout switch is for (see
    /// [`here`]).
    pub(crate) fn shelf_here(&self) -> Option<String> {
        let (rows, _) = self.shelf_rows();
        here(&rows, self.shelf_at().as_ref())
    }

    /// The three small icons on the folders' heading that lay out
    /// `folder`, the one in use lit in the next tint; dim, and pressing
    /// them does nothing, while no folder is open.
    fn layout_switch(&self, folder: Option<String>) -> Element<'_, Message> {
        let p = self.palette;
        let lit = folder
            .as_deref()
            .map(|folder| self.shelf_layouts.of(folder));
        row(Layout::ALL.map(|layout| {
            let on = lit == Some(layout);
            let ink = match (on, &folder) {
                (true, _) => p.primary,
                (false, Some(_)) => p.outline,
                (false, None) => Palette::tint(p.outline, 0.38),
            };
            let tone = picked_tone(on);
            let pressed = button(
                container(look::icon(layout.icon(), 13.0, ink))
                    .center_x(Fill)
                    .center_y(Fill),
            )
            .width(22)
            .height(20)
            .padding(0)
            .on_press_maybe(folder.as_ref().map(|_| Message::ShelfLayout(layout)))
            .style(move |theme, status| {
                let mut style = look::row(p, tone)(theme, status);
                style.border.radius = look::THUMB_RADIUS.into();
                style
            });
            let about = match &folder {
                Some(folder) => layout.about(folder),
                None => "Open a folder to choose how it shows".to_string(),
            };
            hinted(p, pressed, about, tooltip::Position::Bottom)
        }))
        .spacing(2)
        .into()
    }

    /// What `file`, at `note` in its folder, says of itself.
    fn facts(&self, file: &Resource, note: &str, planned: &HashSet<&Path>) -> Facts {
        let picked =
            self.shelf_pick.as_ref() == Some(&file.id) || self.shelf_many.contains(&file.id);
        let known = self
            .shelf_seen
            .contains(&file.id)
            .then(|| self.known_of(file));
        let in_plan = matches!(
            &file.location,
            Location::File(path) if planned.contains(path.as_path())
        );
        let p = self.palette;
        // That it would look soft on the stage's screen is the ⚠ on its
        // picture: the line stays what it is.
        let (under, colour) = match in_plan {
            true => ("○ in the plan".to_string(), p.primary),
            false => (
                Some(note)
                    .filter(|note| !note.is_empty())
                    .into_iter()
                    .chain(kind_word(file))
                    .collect::<Vec<_>>()
                    .join(" · "),
                p.on_surface_variant,
            ),
        };
        Facts {
            picked,
            known,
            in_plan,
            under,
            colour,
        }
    }

    /// `picture` of `file`. Its picture and its length are asked for only
    /// once it comes near sight: an open folder of six hundred files asks
    /// for none of those far out of it.
    fn near_sight<'a>(
        &self,
        file: &Resource,
        picture: Element<'a, Message>,
    ) -> Element<'a, Message> {
        if self.shelf_seen.contains(&file.id) {
            return picture;
        }
        let id = file.id.clone();
        sensor(picture)
            .key(file.id.clone())
            .anticipate(SIGHT)
            .on_show(move |_| Message::ShelfSeen(id.clone()))
            .into()
    }

    /// `file`, `pressed`: pulled, it is carried into the plan, and the
    /// pointer says it can be.
    fn held<'a>(
        &self,
        file: &Resource,
        pressed: button::Button<'a, Message>,
    ) -> Element<'a, Message> {
        mouse_area(pressed)
            .on_enter(Message::ShelfHover(Grip::File(file.id.clone())))
            .on_exit(Message::ShelfLeft(Grip::File(file.id.clone())))
            .interaction(iced::mouse::Interaction::Grab)
            .into()
    }

    /// The files at `run` of `tree`, as tiles.
    fn tiles(
        &self,
        tree: &[ShelfRow],
        run: std::ops::Range<usize>,
        planned: &HashSet<&Path>,
    ) -> Vec<Tile> {
        run.filter_map(|at| match &tree[at] {
            ShelfRow::File { file, note, .. } => Some(Tile {
                at,
                file: file.clone(),
                facts: self.facts(file, note, planned),
            }),
            ShelfRow::Folder { .. } => None,
        })
        .collect()
    }

    /// `tiles` `depth` folders in, as a grid: [`gallery::COLUMNS`] even
    /// 16:9 tiles to a row as wide as the library, each with its name and
    /// its line under its picture.
    fn grid(&self, tiles: Vec<Tile>, depth: usize) -> Element<'_, Message> {
        let p = self.palette;
        let grid = responsive(move |room| {
            let columns = gallery::COLUMNS as f32;
            let wide = (room.width - GRID_GAP * (columns - 1.0)) / columns;
            let lines = tiles.chunks(gallery::COLUMNS).map(|line| {
                row(line.iter().map(|tile| {
                    let Tile { at, file, facts } = tile;
                    let picture = self.row_picture(&facts.thumb(file), wide - 2.0 * TILE_ROOM);
                    let words = column![
                        single_line(file.title.clone())
                            .size(BODY)
                            .font(if facts.picked { BOLD } else { MEDIUM })
                            .color(if facts.picked {
                                p.primary
                            } else {
                                p.on_surface
                            }),
                        single_line(facts.under.clone())
                            .size(META)
                            .color(facts.colour),
                    ]
                    .spacing(1);
                    let pressed = button(
                        container(column![self.near_sight(file, picture), words].spacing(4))
                            .clip(true),
                    )
                    .width(wide)
                    .padding(TILE_ROOM)
                    .on_press(Message::Click(Clicked::Shelf(file.id.clone())))
                    .style(look::row(p, picked_tone(facts.picked)));
                    track::seen(Key::Shelf(*at), &self.places, self.held(file, pressed))
                }))
                .spacing(GRID_GAP)
                .into()
            });
            column(lines).spacing(GRID_GAP).into()
        })
        .height(Length::Shrink);
        // Each picture where a row's would be: the tiles' fills reach
        // round them.
        container(grid)
            .padding(
                Padding::default()
                    .left(8.0 + 18.0 * depth as f32 - TILE_ROOM)
                    .right(8.0 - TILE_ROOM),
            )
            .into()
    }

    /// `tiles` `depth` folders in, at their own shapes: rows as wide as
    /// the library, packed edge to edge, about [`gallery::ROW`] tall. A
    /// file's shape is its picture's, once that is made; 16:9 till then.
    fn shapes(&self, tiles: Vec<Tile>, depth: usize) -> Element<'_, Message> {
        let p = self.palette;
        let shapes = responsive(move |room| {
            let shapes: Vec<f32> = tiles
                .iter()
                .map(|Tile { file, facts, .. }| {
                    facts
                        .known
                        .and_then(|_| self.thumbs.whole(file))
                        // A sliver too thin to press is cut to a quarter.
                        .map_or(16.0 / 9.0, |(_, shape)| shape.clamp(0.25, 4.0))
                })
                .collect();
            let lines = gallery::pack(&shapes, room.width, gallery::ROW, SHAPES_GAP)
                .into_iter()
                .map(|line| {
                    row(line.tiles.map(|n| {
                        let Tile { at, file, facts } = &tiles[n];
                        let size = (shapes[n] * line.height, line.height);
                        let picture =
                            self.tile_picture(&facts.thumb(file), size, &file.title, facts.in_plan);
                        let hovered = matches!(
                            &self.shelf_hover,
                            Some(Grip::File(id)) if *id == file.id
                        );
                        let ring = match (facts.picked, hovered) {
                            (true, _) => Some(p.primary),
                            (false, true) => Some(Palette::tint(p.on_surface, 0.5)),
                            (false, false) => None,
                        };
                        let picture = match ring {
                            Some(ring) => stack![
                                picture,
                                container(Space::new())
                                    .width(Fill)
                                    .height(Fill)
                                    .style(move |_| container::Style {
                                        border: iced::Border {
                                            color: ring,
                                            width: 2.0,
                                            radius: look::THUMB_RADIUS.into(),
                                        },
                                        ..container::Style::default()
                                    }),
                            ]
                            .into(),
                            None => picture,
                        };
                        let pressed = button(self.near_sight(file, picture))
                            .width(size.0)
                            .height(size.1)
                            .padding(0)
                            .on_press(Message::Click(Clicked::Shelf(file.id.clone())))
                            .style(|_, _| button::Style::default());
                        track::seen(Key::Shelf(*at), &self.places, self.held(file, pressed))
                    }))
                    .spacing(SHAPES_GAP)
                    .into()
                });
            column(lines).spacing(SHAPES_GAP).into()
        })
        .height(Length::Shrink);
        container(shapes)
            .padding(
                Padding::default()
                    .left(8.0 + 18.0 * depth as f32)
                    .right(8.0),
            )
            .into()
    }
}

/// A row or tile picked, or a switch lit, is in the next tint.
fn picked_tone(picked: bool) -> Tone {
    match picked {
        true => Tone::Selected,
        false => Tone::Plain,
    }
}

impl App {
    /// The library's find box, over the library and the other lenses, in
    /// a panel `wide`: what is typed in it filters the library, which it
    /// brings back. Its whole hint only where it fits beside the lenses:
    /// a panel as wide as the library is at a laptop's width.
    pub(crate) fn find_box(&self, wide: f32) -> Element<'_, Message> {
        let p = self.palette;
        let hint = match wide >= 260.0 {
            true => "Find something to add…",
            false => "Find something…",
        };
        iced::widget::stack![
            text_input(hint, &self.shelf_filter)
                .id(FILTER)
                .on_input(Message::ShelfFilter)
                .size(META)
                .padding(Padding::from([8, 12]).left(36.0))
                .style(look::find(p)),
            container(look::icon(Icon::Search, 15.0, p.on_surface_variant))
                .padding(Padding::default().left(13.0))
                .center_y(Fill),
        ]
        .into()
    }

    /// The library's own group: Lantern's sources the searcher has, then
    /// a live slot and a black screen.
    pub(crate) fn own_rows(&self) -> Vec<Own> {
        let steps = [
            (
                NewStep::LiveSlot,
                "Live",
                "A live slot: show anything as it comes",
            ),
            (
                NewStep::BlackScreen,
                "Black screen",
                "Nothing on the projector",
            ),
        ]
        .map(|(new, name, about)| Own {
            icon: new_icon(new),
            name,
            detail: String::new(),
            about: about.to_string(),
            does: Does::Add(new),
        });
        let mut rows = own_sources(&self.engine.0.searcher().bangs());
        rows.extend(steps);
        rows
    }

    /// What a press on the library's row named `name` does, as a hand's
    /// would: its own group's first, then the tree's rows in sight.
    pub(crate) fn library_press(&self, name: &str) -> Option<Message> {
        let own = self
            .own_rows()
            .into_iter()
            .find(|own| own.name.eq_ignore_ascii_case(name))
            .map(|own| own.press());
        own.or_else(|| {
            let (tree, _) = self.shelf_rows();
            tree.iter()
                .find(|row| row.name().eq_ignore_ascii_case(name))
                .map(ShelfRow::press)
        })
    }

    /// A row of the library's own group: its icon in the primary colour,
    /// its name, and what it says quietly on the right.
    fn own_row(&self, own: Own) -> Element<'_, Message> {
        let p = self.palette;
        let can = match own.does {
            Does::Search(_) => true,
            Does::Add(_) => self.can_edit(),
        };
        let press = can.then(|| own.press());
        let mut line = row![
            look::icon(own.icon, 16.0, p.primary),
            container(single_line(own.name).size(BODY).font(MEDIUM))
                .width(Fill)
                .clip(true),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if !own.detail.is_empty() {
            line = line.push(
                container(
                    single_line(own.detail)
                        .size(META)
                        .color(p.on_surface_variant),
                )
                .max_width(110)
                .clip(true),
            );
        }
        let pressed = button(line)
            .width(Fill)
            .padding([8, 8])
            .on_press_maybe(press)
            .style(look::row(p, Tone::Plain));
        let tipped = hinted(p, pressed, own.about, tooltip::Position::Right);
        let Does::Add(new) = own.does else {
            return tipped;
        };
        let held = mouse_area(tipped)
            .on_enter(Message::ShelfHover(Grip::New(new)))
            .on_exit(Message::ShelfLeft(Grip::New(new)));
        match can {
            true => held.interaction(iced::mouse::Interaction::Grab).into(),
            false => held.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use lantern_engine::{Action, Bang, Kind, Location, Resource, ResourceId, SourceId};

    use super::{Carried, Go, Held, Layout, Layouts, ShelfAt, ShelfRow, here, own_sources};
    use crate::Message;
    use crate::plan_edit::NewStep;

    fn file(title: &str) -> Arc<Resource> {
        let source = SourceId::new("test").unwrap();
        Arc::new(Resource::new(
            ResourceId::new(&source, title),
            title,
            Kind::Image,
            Location::Url(title.into()),
        ))
    }

    #[test]
    fn files_let_go_are_each_shown_in_their_order() {
        let mut carried = Carried::new(Held::File(file("Psalm 23")));
        carried.more = vec![file("Amazing Grace")];
        let steps = carried.steps();
        let titles: Vec<_> = steps.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(titles, ["Psalm 23", "Amazing Grace"]);
        assert!(
            matches!(&steps[1].1, Action::Show { resource } if resource.title == "Amazing Grace")
        );
        assert_eq!(carried.title(), "Psalm 23");
        assert!(carried.file().is_some());
    }

    #[test]
    fn a_step_the_library_always_has_lets_go_as_that_step() {
        let carried = Carried::new(Held::New(NewStep::BlackScreen));
        let steps = carried.steps();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].0, "Black screen");
        assert!(matches!(steps[0].1, Action::Blank));
        assert_eq!(carried.title(), "Black screen");
        assert!(carried.file().is_none(), "no file to look at");

        let live = Carried::new(Held::New(NewStep::LiveSlot));
        assert!(matches!(live.steps()[0].1, Action::Live { .. }));
    }

    #[test]
    fn the_library_lists_lantern_s_sources_there_are_in_its_order() {
        let bang = |bang: &str, sources: &[&str]| Bang {
            bang: bang.into(),
            sources: sources.iter().map(|source| source.to_string()).collect(),
            on_request: false,
        };
        // As the searcher gives them, by bang, a folder's own among them.
        let bangs = [
            bang("b", &["RV1909", "KJV"]),
            bang("f", &["Local folders"]),
            bang("gi", &["Images (Brave)"]),
            bang("h", &["Hymns"]),
            bang("t", &["Time"]),
            bang("x", &["Text"]),
            bang("yt", &["YouTube"]),
        ];
        let rows = own_sources(&bangs);
        let names: Vec<&str> = rows.iter().map(|own| own.name).collect();
        assert_eq!(names, ["Bible", "YouTube", "Images", "Text", "Time"]);
        assert_eq!(rows[0].detail, "RV1909, KJV", "a Bible says which");
        assert!(
            rows[1..].iter().all(|own| own.detail.is_empty()),
            "the others are named for what they are"
        );
        assert!(
            matches!(rows[0].press(), Message::Bang(b) if b == "b"),
            "it opens the search on its bang"
        );
        let without_a_bible = own_sources(&bangs[1..]);
        assert!(without_a_bible.iter().all(|own| own.name != "Bible"));
    }

    #[test]
    fn the_layout_switch_is_for_the_open_folder_in_use() {
        let folder = |key: &str, depth: usize, open: bool| ShelfRow::Folder {
            label: key.rsplit('/').next().unwrap_or(key).to_string(),
            key: key.to_string(),
            count: 1,
            depth,
            open,
        };
        let item = |title: &str, depth: usize| ShelfRow::File {
            file: file(title),
            depth,
            note: String::new(),
        };
        let rows = [
            folder("Hymns", 0, true),
            folder("Hymns/Kids", 1, true),
            item("Jesus loves me", 2),
            item("Amazing Grace", 1),
            folder("Pictures", 0, true),
            item("Easter", 1),
            folder("Videos", 0, false),
        ];
        let on = |title: &str| Some(ShelfAt::File(file(title).id.clone()));
        let in_use = |at: Option<ShelfAt>| here(&rows, at.as_ref());
        assert_eq!(
            in_use(on("Jesus loves me")),
            Some("Hymns".into()),
            "a file deeper in"
        );
        assert_eq!(in_use(on("Easter")), Some("Pictures".into()));
        assert_eq!(
            in_use(Some(ShelfAt::Folder("Hymns/Kids".into()))),
            Some("Hymns".into()),
            "a folder inside one"
        );
        assert_eq!(
            in_use(None),
            Some("Pictures".into()),
            "with nothing picked, the last one open"
        );
        assert_eq!(
            in_use(Some(ShelfAt::Folder("Videos".into()))),
            Some("Pictures".into()),
            "a closed folder has nothing to lay out"
        );
        assert_eq!(here(&rows[6..], None), None, "every folder is closed");
    }

    #[test]
    fn each_folder_keeps_its_layout_for_the_next_start() {
        let data = tempfile::tempdir().unwrap();
        let mut layouts = Layouts::read(data.path());
        assert_eq!(
            layouts.of("Pictures"),
            Layout::List,
            "a list till given another"
        );
        layouts.set("Pictures", Layout::Shapes).unwrap();
        layouts.set("Hymns", Layout::Grid).unwrap();
        layouts.set("Hymns", Layout::List).unwrap();
        let again = Layouts::read(data.path());
        assert_eq!(
            (again.of("Pictures"), again.of("Hymns")),
            (Layout::Shapes, Layout::List)
        );
        std::fs::write(
            data.path().join("library-layouts"),
            "Pictures\tgrid\nnonsense\nVideos\ttiles\n",
        )
        .unwrap();
        let read = Layouts::read(data.path());
        assert_eq!(
            (read.of("Pictures"), read.of("Videos")),
            (Layout::Grid, Layout::List),
            "a line that can't be read is left out"
        );
    }

    #[test]
    fn the_arrows_go_round_and_home_and_end_go_to_the_ends() {
        assert_eq!(Go::Next.from(None, 3), Some(0));
        assert_eq!(Go::Prev.from(None, 3), Some(2));
        assert_eq!(Go::Next.from(Some(2), 3), Some(0), "round to the top");
        assert_eq!(Go::Prev.from(Some(0), 3), Some(2), "round to the end");
        assert_eq!(Go::First.from(Some(2), 3), Some(0));
        assert_eq!(Go::Last.from(Some(0), 3), Some(2));
        assert_eq!(Go::Last.from(None, 0), None, "nothing to be on");
    }
}
