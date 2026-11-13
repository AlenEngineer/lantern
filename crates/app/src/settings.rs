//! The settings view (Ctrl+,): what `config.toml` holds, as fields. Save
//! writes the file and applies it at once: the library swaps its sources
//! and reindexes, and the colours reload.

use std::path::Path;

use iced::widget::{Space, button, column, row, text, text_input};
use iced::{Alignment, Element, Fill};

use crate::config::{self, BibleEntry, Config, FolderEntry, Lang};
use crate::dashboard::{chip, pill, scrolling, wide};
use crate::keys::Op;
use crate::look::{self, BODY, BOLD, HEADING, Icon, MEDIUM, META, PILL, Tone};
use crate::palette::Palette;
use crate::{App, Message};

const LABEL_FIELD: f32 = 160.0;

/// The settings as they're being edited.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    folders: Vec<Folder>,
    bibles: Vec<Bible>,
    brave_key: String,
    colors: String,
    /// What the last save did.
    pub notice: Option<Result<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Folder {
    label: String,
    path: String,
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
            notice: None,
        }
    }

    pub fn edit(&mut self, edit: Edit) {
        self.notice = None;
        match edit {
            Edit::FolderLabel(i, text) => self.folder(i, |f| f.label = text),
            Edit::FolderPath(i, text) => self.folder(i, |f| f.path = text),
            Edit::AddFolder => self.folders.push(Folder::default()),
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
        Ok(Config {
            brave_api_key: optional(&self.brave_key),
            colors: optional(&self.colors).map(|c| path(&c)),
            folders,
            bibles,
        })
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
        let header = row![
            look::icon(Icon::Settings, 20.0, p.primary),
            text("Settings").size(HEADING).font(BOLD).color(p.primary),
            chip(p, &shown_file, Tone::Plain),
            Space::new().width(Fill),
            text("Esc closes").size(META).color(p.on_surface_variant),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .height(36);

        let mut folders = column![section(
            p,
            "Media folders",
            "Lantern finds every file it can show in these folders, and in the folders inside them."
        )]
        .spacing(6);
        for (i, folder) in draft.folders.iter().enumerate() {
            folders = folders.push(
                row![
                    field(p, "Label", &folder.label, move |t| Edit::FolderLabel(i, t))
                        .width(LABEL_FIELD),
                    field(p, "~/Music", &folder.path, move |t| Edit::FolderPath(i, t)).width(Fill),
                    remove(p, Edit::RemoveFolder(i)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
        folders = folders.push(add(p, "Add folder", Edit::AddFolder));

        let mut bibles = column![section(
            p,
            "Bibles",
            "One .json or .tsv file per translation; the language is that of its book names."
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
                    field(p, "Name", &bible.name, move |t| Edit::BibleName(i, t))
                        .width(LABEL_FIELD),
                    lang("es", Lang::Es),
                    lang("en", Lang::En),
                    field(p, "~/bibles/rv1909.tsv", &bible.path, move |t| {
                        Edit::BiblePath(i, t)
                    })
                    .width(Fill),
                    remove(p, Edit::RemoveBible(i)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
        bibles = bibles.push(add(p, "Add Bible", Edit::AddBible));

        let images = column![
            section(
                p,
                "Image search",
                "!gi needs a Brave Search API key, from brave.com/search/api. It's billed to you, so keep this file private."
            ),
            field(p, "Brave Search API key", &draft.brave_key, Edit::BraveKey).secure(true),
        ]
        .spacing(6);
        let colors = column![
            section(
                p,
                "Colours",
                "A Matugen colours file, so Lantern follows your desktop's theme. Empty uses Lantern's own."
            ),
            field(p, "~/.config/amane/colors.json", &draft.colors, Edit::Colors),
        ]
        .spacing(6);

        let body = scrolling(column![folders, bibles, images, colors].spacing(24));
        let mut footer = row![].spacing(8).align_y(Alignment::Center);
        if let Some(notice) = &draft.notice {
            let (message, color) = match notice {
                Ok(done) if self.library.0.indexing() => {
                    (format!("{done} Reindexing…"), p.on_surface_variant)
                }
                Ok(done) => (done.clone(), p.on_surface_variant),
                Err(why) => (why.clone(), p.error),
            };
            footer = footer.push(text(message).size(META).color(color).width(Fill));
        } else {
            footer = footer.push(Space::new().width(Fill));
        }
        footer = footer
            .push(pill(
                p,
                "Close · Esc",
                Tone::Plain,
                PILL,
                Some(Message::Key {
                    op: Op::ClearSearch,
                    only_if_empty: false,
                }),
            ))
            .push(pill(
                p,
                "Save · Ctrl+S",
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

        draft.edit(Edit::RemoveFolder(1));
        draft.edit(Edit::AddBible);
        draft.edit(Edit::BiblePath(1, "~/kjv.json".into()));
        draft.edit(Edit::BibleLang(1, Lang::En));
        let config = draft.config(dir, home()).unwrap();
        assert_eq!(config.bibles[1].path, Path::new("/home/ana/kjv.json"));
        assert_eq!(config.bibles[1].lang, Lang::En);
    }
}
