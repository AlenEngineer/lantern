//! The settings view (Ctrl+,): what `config.toml` holds, as fields. Save
//! writes the file and applies it at once: the library swaps its sources
//! and reindexes, and the colours reload. Path fields suggest what's on
//! disk as they're typed (see [`crate::paths`]).

use std::path::{Path, PathBuf};

use iced::widget::{Space, button, column, row, text, text_input};
use iced::{Alignment, Element, Fill};

use lantern_engine::{Look, Rgb, fold};

use crate::config::{self, BibleEntry, Config, FolderEntry, Lang};
use crate::dashboard::{chip, pill, scrolling, wide};
use crate::fit;
use crate::keys::Op;
use crate::look::{self, BODY, BOLD, HEADING, Icon, MEDIUM, META, PILL, Tone};
use crate::palette::Palette;
use crate::paths::{self, Suggestion, Want};
use crate::{App, Message};

const LABEL_FIELD: f32 = 160.0;
const SMALL_FIELD: f32 = 110.0;
const BANG_FIELD: f32 = 80.0;
/// The settings' scrolling body.
pub const SCROLL: &str = "settings-body";
const SAMPLE_WIDTH: f32 = 384.0;

/// The settings as they're being edited.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    folders: Vec<Folder>,
    bibles: Vec<Bible>,
    brave_key: String,
    colors: String,
    stage: Stage,
    /// The font families installed, for the font field to suggest.
    fonts: Vec<String>,
    /// What the last save did.
    pub notice: Option<Result<String, String>>,
    home: Option<PathBuf>,
    /// The path field being typed in, and what it could become.
    suggesting: Option<(Field, Vec<Suggestion>)>,
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
}

/// A field that suggestions fill: a path, or the stage's font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Folder(usize),
    Bible(usize),
    Colors,
    StageImage,
    Font,
}

impl Field {
    /// What a path field takes; `None` for the font.
    fn want(self) -> Option<Want> {
        Some(match self {
            Field::Folder(_) => Want::Folders,
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
    Background(String),
    StageImage(String),
    TextColor(String),
    Font(String),
    MaxSize(String),
    MinSize(String),
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
            stage: Stage {
                background: config.stage.background.to_string(),
                image: config.stage.image.as_deref().map(shown).unwrap_or_default(),
                text: config.stage.text.to_string(),
                font: config.stage.font.clone().unwrap_or_default(),
                max_size: config.stage.max_size.to_string(),
                min_size: config.stage.min_size.to_string(),
            },
            fonts: Vec::new(),
            notice: None,
            home: home.map(Path::to_path_buf),
            suggesting: None,
        }
    }

    /// The draft, with these font families to suggest.
    pub fn with_fonts(self, fonts: Vec<String>) -> Draft {
        Draft { fonts, ..self }
    }

    /// Applies `edit`. Returns the path field it filled in, if any, so the
    /// view can put the cursor at its end.
    pub fn edit(&mut self, edit: Edit) -> Option<Field> {
        self.notice = None;
        let typed = match &edit {
            Edit::FolderPath(i, _) => Some(Field::Folder(*i)),
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
            Field::Bible(i) => self.bibles.get(i).map_or("", |b| b.path.as_str()),
            Field::Colors => &self.colors,
            Field::StageImage => &self.stage.image,
            Field::Font => &self.stage.font,
        }
    }

    fn set_path(&mut self, field: Field, path: String) {
        match field {
            Field::Folder(i) => self.folder(i, |f| f.path = path),
            Field::Bible(i) => self.bible(i, |b| b.path = path),
            Field::Colors => self.colors = path,
            Field::StageImage => self.stage.image = path,
            Field::Font => self.stage.font = path,
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
            Edit::Background(text) => self.stage.background = text,
            Edit::StageImage(text) => self.stage.image = text,
            Edit::TextColor(text) => self.stage.text = text,
            Edit::Font(text) => self.stage.font = text,
            Edit::MaxSize(text) => self.stage.max_size = text,
            Edit::MinSize(text) => self.stage.min_size = text,
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
        let stage = Look {
            background: colour(&self.stage.background, "background")?,
            image: optional(&self.stage.image).map(|i| path(&i)),
            text: colour(&self.stage.text, "text colour")?,
            font: optional(&self.stage.font),
            max_size: size(&self.stage.max_size, "largest size")?,
            min_size: size(&self.stage.min_size, "smallest size")?,
        };
        Ok(Config {
            brave_api_key: optional(&self.brave_key),
            colors: optional(&self.colors).map(|c| path(&c)),
            folders,
            bibles,
            stage,
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
            "Lantern finds every file it can show in these folders, and in the folders inside them. \
             Give a folder a bang, like !h, to search only it: !h himno. !f searches every folder."
        )]
        .spacing(6);
        for (i, folder) in draft.folders.iter().enumerate() {
            folders = folders.push(
                row![
                    field(p, "Label", &folder.label, move |t| Edit::FolderLabel(i, t))
                        .width(LABEL_FIELD),
                    field(p, "~/Music", &folder.path, move |t| Edit::FolderPath(i, t))
                        .id(Field::Folder(i).id())
                        .width(Fill),
                    field(p, "!bang", &folder.bang, move |t| Edit::FolderBang(i, t))
                        .width(BANG_FIELD),
                    remove(p, Edit::RemoveFolder(i)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
            folders = folders.push(suggestions(p, draft, Field::Folder(i)));
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
                    .id(Field::Bible(i).id())
                    .width(Fill),
                    remove(p, Edit::RemoveBible(i)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
            bibles = bibles.push(suggestions(p, draft, Field::Bible(i)));
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
            field(p, "~/.config/amane/colors.json", &draft.colors, Edit::Colors)
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
        let stage = column![
            section(
                p,
                "Stage",
                "What the audience sees behind the content, and how text is set: as large as fits, between the two sizes."
            ),
            line(row![
                label("Colours"),
                note("background"),
                small("#000000", &draft.stage.background, Edit::Background),
                swatch(&draft.stage.background),
                Space::new().width(12),
                note("text"),
                small("#ffffff", &draft.stage.text, Edit::TextColor),
                swatch(&draft.stage.text),
            ]),
            line(row![
                label("Text size"),
                small("40", &draft.stage.min_size, Edit::MinSize),
                note("to"),
                small("96", &draft.stage.max_size, Edit::MaxSize),
                note("points on a 1080-line screen"),
            ]),
            line(row![
                label("Image"),
                field(p, "none: ~/Pictures/cross.jpg", &draft.stage.image, Edit::StageImage)
                    .id(Field::StageImage.id())
                    .width(Fill),
            ]),
            suggestions(p, draft, Field::StageImage),
            line(row![
                label("Font"),
                field(p, "the system's own: type to find one", &draft.stage.font, Edit::Font)
                    .id(Field::Font.id())
                    .width(Fill),
            ]),
            suggestions(p, draft, Field::Font),
            row![Space::new().width(LABEL_FIELD), self.stage_sample(draft)].spacing(8),
        ]
        .spacing(6);

        let body =
            scrolling(column![folders, bibles, stage, images, colors].spacing(24)).id(SCROLL);
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

impl App {
    /// A small stage with the look as typed, showing a verse, so changes
    /// show before they're saved. Fields that don't read yet keep the
    /// saved look's value.
    fn stage_sample<'a>(&self, draft: &Draft) -> Element<'a, Message> {
        let saved = &self.config.stage;
        let look = Look {
            background: draft.stage.background.parse().unwrap_or(saved.background),
            text: draft.stage.text.parse().unwrap_or(saved.text),
            image: Some(draft.stage.image.trim())
                .filter(|i| !i.is_empty())
                .and_then(|i| {
                    let home = self.home.as_deref();
                    let dir = self.config_status.path().parent()?;
                    Some(config::resolve(Path::new(i), dir, home))
                }),
            font: Some(draft.stage.font.trim().to_string()).filter(|f| !f.is_empty()),
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
        };
        let sample = crate::backdrop(
            &look,
            fit::fit_text(
                "Porque de tal manera amó Dios al mundo,\nque ha dado á su Hijo unigénito",
                &look,
            ),
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
    iced::widget::container(list)
        .padding(iced::Padding::default().left(LABEL_FIELD + 6.0))
        .into()
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
        assert_eq!(draft.stage.font, "DejaVu Serif");
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
    }
}
