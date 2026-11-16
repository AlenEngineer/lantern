//! The library, beside the plan while planning: the media folders as a
//! tree, to build a plan by browsing as well as by searching. A click
//! looks at a file, on this screen only; a double click or Enter adds it
//! to the plan, after the step that is selected.

use std::sync::Arc;

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Fill};
use lantern_engine::{Location, Query, Resource};

use crate::dashboard::{scrolling, single_line};
use crate::look::{self, BODY, BOLD, Icon, META, Tone};
use crate::{App, Clicked, Message};

/// The most files a folder lists. Past it the search is the way in: a
/// folder of six hundred hymns is found by number, not by scrolling.
const MOST: usize = 200;

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

    /// The library as a tree: each folder, and under the open one its
    /// files.
    pub(crate) fn shelf_pane(&self) -> Element<'_, Message> {
        let p = self.palette;
        let mut rows = column![
            container(
                text("LIBRARY")
                    .size(META)
                    .font(BOLD)
                    .color(p.on_surface_variant)
            )
            .padding([4, 8])
        ]
        .spacing(2);
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
        for folder in &self.shelf {
            let open = self.shelf_open.as_deref() == Some(folder.label.as_str());
            rows = rows.push(
                button(
                    row![
                        text(if open { "▾" } else { "▸" })
                            .size(META)
                            .color(p.on_surface_variant),
                        look::icon(Icon::Folder, 14.0, p.on_surface_variant),
                        container(
                            single_line(folder.label.clone())
                                .size(BODY)
                                .font(BOLD)
                                .color(if open { p.primary } else { p.on_surface })
                        )
                        .width(Fill)
                        .clip(true),
                        text(folder.files.len().to_string())
                            .size(META)
                            .color(p.on_surface_variant),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .width(Fill)
                .padding([6, 8])
                .on_press(Message::ShelfFold(folder.label.clone()))
                .style(look::row(p, Tone::Plain)),
            );
            if !open {
                continue;
            }
            for file in folder.files.iter().take(MOST) {
                let picked = self.shelf_pick.as_ref() == Some(&file.id);
                let colour = if picked { p.primary } else { p.on_surface };
                // Where in the folder it is, when it is in a sub-folder.
                let within = file.tags[1..].join(" / ");
                let mut line = row![
                    look::icon(Icon::of(&file.kind), 13.0, colour),
                    container(single_line(file.title.clone()).size(BODY).color(colour))
                        .width(Fill)
                        .clip(true),
                ]
                .spacing(8)
                .align_y(Alignment::Center);
                if !within.is_empty() {
                    line = line.push(
                        container(single_line(within).size(META).color(p.on_surface_variant))
                            .max_width(90)
                            .clip(true),
                    );
                }
                rows = rows.push(
                    button(line)
                        .width(Fill)
                        .padding(
                            iced::Padding::default()
                                .left(30.0)
                                .right(8.0)
                                .top(5.0)
                                .bottom(5.0),
                        )
                        .on_press(Message::Click(Clicked::Shelf(file.id.clone())))
                        .style(look::row(
                            p,
                            if picked { Tone::Selected } else { Tone::Plain },
                        )),
                );
            }
            if folder.files.len() > MOST {
                rows = rows.push(
                    container(
                        text(format!(
                            "and {} more: search finds them",
                            folder.files.len() - MOST
                        ))
                        .size(META)
                        .color(p.on_surface_variant),
                    )
                    .padding(iced::Padding::default().left(30.0).top(4.0).bottom(4.0)),
                );
            }
        }
        column![scrolling(rows), Space::new().height(4)]
            .width(Fill)
            .height(Fill)
            .into()
    }
}
