//! The library, beside the plan while planning: the media folders as a
//! tree, to build a plan by browsing as well as by searching. A click
//! looks at a file, on this screen only; a double click or Enter adds it
//! to the plan, after the step that is selected.
//!
//! The keyboard walks it too, once the arrows are given to it: ↑ ↓ move
//! through the rows in sight, → and ← open and close folders.

use std::sync::Arc;

use iced::widget::{
    Space, button, column, container, operation, row, scrollable, text, text_input,
};
use iced::{Alignment, Element, Fill, Task};
use lantern_engine::{Location, Query, Resource};

use crate::dashboard::{scrolling, single_line};
use crate::look::{self, BODY, BOLD, Icon, META, Tone};
use crate::{App, Clicked, Message};

/// The most rows the tree draws at once. A folder of six hundred hymns
/// is all there, a filter away; past this many, the filter is the way.
const MOST: usize = 1000;

/// The id of the library's filter box, to focus it.
pub const FILTER: &str = "library-filter";

/// The id of the tree's rows, to scroll them.
pub const ROWS: &str = "library-rows";

/// The folders between a file and its library folder, as a path:
/// `Hymns/Kids`. Empty for a file right in the folder.
fn within(file: &Resource) -> String {
    file.tags.get(1..).unwrap_or_default().join("/")
}

/// A file of the library held by the pointer, on its way into the plan.
pub struct Carried {
    pub file: Arc<Resource>,
    /// Where it was picked up, and where the pointer is now.
    from: Option<iced::Point>,
    pub at: iced::Point,
    /// It has moved far enough to be a drag and not a click.
    pub moving: bool,
}

impl Carried {
    pub fn new(file: Arc<Resource>) -> Carried {
        Carried {
            file,
            from: None,
            at: iced::Point::ORIGIN,
            moving: false,
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
        self.in_library && self.mode == crate::mode::Mode::Plan && self.adding.is_none()
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

    /// The library as a tree: each folder, the folders inside it, and
    /// its files.
    pub(crate) fn shelf_pane(&self) -> Element<'_, Message> {
        let p = self.palette;
        let keys = self.keys_in_library();
        let (tree, left_out) = self.shelf_rows();
        let mut rows = column![].spacing(2);
        if self.shelf.is_empty() {
            rows = rows.push(
                container(
                    text("No media folders yet. Add them in Settings › Library.")
                        .size(BODY)
                        .color(p.on_surface_variant),
                )
                .padding([4, 8]),
            );
        }
        let folder_row = |label: String, path: String, count: usize, depth: usize, open: bool| {
            let at = keys && self.shelf_folder.as_ref() == Some(&path);
            button(
                row![
                    text(if open { "▾" } else { "▸" })
                        .size(META)
                        .color(p.on_surface_variant),
                    look::icon(Icon::Folder, 14.0, p.on_surface_variant),
                    container(single_line(label).size(BODY).font(BOLD).color(if open {
                        p.primary
                    } else {
                        p.on_surface
                    }))
                    .width(Fill)
                    .clip(true),
                    text(count.to_string())
                        .size(META)
                        .color(p.on_surface_variant),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .width(Fill)
            .padding(
                iced::Padding::default()
                    .left(8.0 + 18.0 * depth as f32)
                    .right(8.0)
                    .top(6.0)
                    .bottom(6.0),
            )
            .on_press(Message::ShelfFold(path))
            .style(look::row(p, if at { Tone::Selected } else { Tone::Plain }))
        };
        let file_row = |file: &Arc<Resource>, depth: usize, note: String| {
            let picked = self.shelf_pick.as_ref() == Some(&file.id);
            let colour = if picked { p.primary } else { p.on_surface };
            let mut line = row![
                look::icon(Icon::of(&file.kind), 13.0, colour),
                container(single_line(file.title.clone()).size(BODY).color(colour))
                    .width(Fill)
                    .clip(true),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            if !note.is_empty() {
                line = line.push(
                    container(single_line(note).size(META).color(p.on_surface_variant))
                        .max_width(120)
                        .clip(true),
                );
            }
            // Pressed and pulled, it is carried into the plan; the pointer
            // says it can be.
            iced::widget::mouse_area(
                button(line)
                    .width(Fill)
                    .padding(
                        iced::Padding::default()
                            .left(12.0 + 18.0 * depth as f32)
                            .right(8.0)
                            .top(5.0)
                            .bottom(5.0),
                    )
                    .on_press(Message::Click(Clicked::Shelf(file.id.clone())))
                    .style(look::row(
                        p,
                        if picked { Tone::Selected } else { Tone::Plain },
                    )),
            )
            .on_enter(Message::ShelfHover(Some(file.id.clone())))
            .on_exit(Message::ShelfHover(None))
            .interaction(iced::mouse::Interaction::Grab)
        };
        let files = tree
            .iter()
            .filter(|row| matches!(row, ShelfRow::File { .. }))
            .count();
        for row in tree {
            rows = match row {
                ShelfRow::Folder {
                    label,
                    key,
                    count,
                    depth,
                    open,
                } => rows.push(folder_row(label, key, count, depth, open)),
                ShelfRow::File { file, depth, note } => rows.push(file_row(&file, depth, note)),
            };
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
        column![
            row![
                // Lit while the arrows are here.
                text("LIBRARY").size(META).font(BOLD).color(if keys {
                    p.primary
                } else {
                    p.on_surface_variant
                }),
                text_input("Filter…", &self.shelf_filter)
                    .id(FILTER)
                    .on_input(Message::ShelfFilter)
                    .size(BODY)
                    .padding([5, 10])
                    .style(look::input(p)),
            ]
            .spacing(10)
            .padding([0, 8])
            .align_y(Alignment::Center),
            scrolling(rows).id(ROWS),
            Space::new().height(4)
        ]
        .spacing(8)
        .width(Fill)
        .height(Fill)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::Go;

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
