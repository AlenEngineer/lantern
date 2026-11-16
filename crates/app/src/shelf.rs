//! The library, beside the plan while planning: the media folders as a
//! tree, to build a plan by browsing as well as by searching. A click
//! looks at a file, on this screen only; a double click or Enter adds it
//! to the plan, after the step that is selected.

use std::sync::Arc;

use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Element, Fill};
use lantern_engine::{Location, Query, Resource};

use crate::dashboard::{scrolling, single_line};
use crate::look::{self, BODY, BOLD, Icon, META, Tone};
use crate::{App, Clicked, Message};

/// The most rows the tree draws at once. A folder of six hundred hymns
/// is all there, a filter away; past this many, the filter is the way.
const MOST: usize = 1000;

/// The id of the library's filter box, to focus it.
pub const FILTER: &str = "library-filter";

/// The folders between a file and its library folder, as a path:
/// `Hymns/Kids`. Empty for a file right in the folder.
fn within(file: &Resource) -> String {
    file.tags.get(1..).unwrap_or_default().join("/")
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

    /// The library as a tree: each folder, the folders inside it, and
    /// its files. With something typed in the filter, the files whose
    /// name or folder has it, wherever they are.
    pub(crate) fn shelf_pane(&self) -> Element<'_, Message> {
        let p = self.palette;
        let filter = lantern_engine::fold(self.shelf_filter.trim());
        let mut rows = column![].spacing(2);
        let mut drawn = 0;
        let mut left_out = 0;
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
            .style(look::row(p, Tone::Plain))
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
                ))
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
                rows = rows.push(folder_row(
                    folder.label.clone(),
                    folder.label.clone(),
                    found.len(),
                    0,
                    true,
                ));
                for file in found {
                    if drawn >= MOST {
                        left_out += 1;
                        continue;
                    }
                    drawn += 1;
                    rows = rows.push(file_row(file, 1, within(file)));
                }
                continue;
            }
            let open = self.shelf_open.contains(&folder.label);
            rows = rows.push(folder_row(
                folder.label.clone(),
                folder.label.clone(),
                folder.files.len(),
                0,
                open,
            ));
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
                rows = rows.push(folder_row(name, key, under, depth, open));
                if !open {
                    continue;
                }
                for file in files_in(path) {
                    if drawn >= MOST {
                        left_out += 1;
                        continue;
                    }
                    drawn += 1;
                    rows = rows.push(file_row(file, depth + 1, String::new()));
                }
            }
            for file in files_in("") {
                if drawn >= MOST {
                    left_out += 1;
                    continue;
                }
                drawn += 1;
                rows = rows.push(file_row(file, 1, String::new()));
            }
        }
        if !filter.is_empty() && drawn == 0 && !self.shelf.is_empty() {
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
                text("LIBRARY")
                    .size(META)
                    .font(BOLD)
                    .color(p.on_surface_variant),
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
            scrolling(rows),
            Space::new().height(4)
        ]
        .spacing(8)
        .width(Fill)
        .height(Fill)
        .into()
    }
}
