//! Where a picked step's picture and sound come from, in its card: its
//! own, another file (the one picked in search or the library), what was
//! on kept, or nothing. And the lines under a row that says so, on the
//! rows that mix.

use std::fmt;
use std::sync::Arc;

use iced::widget::{column, pick_list, row};
use iced::{Alignment, Element};
use lantern_engine::{Action, Kind, Picture, Resource, Step, StepId, StepSound};

use crate::look::{self, Icon, META};
use crate::palette::Palette;
use crate::{App, Message};

/// What a step's picture can be set to.
#[derive(Debug, Clone, PartialEq)]
pub enum PictureChoice {
    Own,
    /// Another picture: the one picked, named.
    File(Option<String>),
    Keep,
    Black,
}

impl fmt::Display for PictureChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PictureChoice::Own => write!(f, "its own"),
            PictureChoice::File(Some(name)) => write!(f, "{name}"),
            PictureChoice::File(None) => write!(f, "the picked picture"),
            PictureChoice::Keep => write!(f, "what was on"),
            PictureChoice::Black => write!(f, "nothing"),
        }
    }
}

/// What a step's sound can be set to.
#[derive(Debug, Clone, PartialEq)]
pub enum SoundChoice {
    Own,
    /// Another sound: the one picked, named.
    File(Option<String>),
    None,
}

impl fmt::Display for SoundChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SoundChoice::Own => write!(f, "its own"),
            SoundChoice::File(Some(name)) => write!(f, "{name}"),
            SoundChoice::File(None) => write!(f, "the picked sound"),
            SoundChoice::None => write!(f, "none"),
        }
    }
}

/// What the step's picture is set to now.
fn picture_now(step: &Step) -> PictureChoice {
    match &step.picture {
        Picture::Own => PictureChoice::Own,
        Picture::File(file) => PictureChoice::File(Some(file.title.clone())),
        Picture::Keep => PictureChoice::Keep,
        Picture::Black => PictureChoice::Black,
    }
}

/// What the step's sound is set to now.
fn sound_now(step: &Step) -> SoundChoice {
    match (&step.sound, step.silent) {
        (Some(sound), _) => SoundChoice::File(Some(sound.resource.title.clone())),
        (None, true) => SoundChoice::None,
        (None, false) => SoundChoice::Own,
    }
}

impl App {
    /// The file picked in search or in the library, if one is.
    fn picked_file(&self) -> Option<Arc<Resource>> {
        self.search
            .picked()
            .cloned()
            .or_else(|| self.shelf_pick.as_ref().and_then(|id| self.shelf_file(id)))
    }

    /// The picked file, when it can be a picture.
    fn picked_picture(&self) -> Option<Arc<Resource>> {
        self.picked_file()
            .filter(|file| matches!(file.kind, Kind::Image | Kind::Document | Kind::Presentation))
    }

    /// The picked file, when it can be heard.
    fn picked_sound(&self) -> Option<Arc<Resource>> {
        self.picked_file()
            .filter(|file| matches!(file.kind, Kind::Audio | Kind::Video))
    }

    /// The two lines of a picked step's card that set where its picture
    /// and its sound come from.
    pub(crate) fn sources_card<'a>(&self, p: Palette, step: &Step) -> Element<'a, Message> {
        let id = step.id;
        let picture = {
            let mut choices = vec![PictureChoice::Own];
            choices.push(PictureChoice::File(
                self.picked_picture().map(|file| file.title.clone()),
            ));
            choices.extend([PictureChoice::Keep, PictureChoice::Black]);
            chooser(p, Icon::Image, choices, picture_now(step), move |choice| {
                Message::StepPicture(id, choice)
            })
        };
        let sound = {
            let choices = vec![
                SoundChoice::Own,
                SoundChoice::File(self.picked_sound().map(|file| file.title.clone())),
                SoundChoice::None,
            ];
            chooser(p, Icon::Music, choices, sound_now(step), move |choice| {
                Message::StepSoundFrom(id, choice)
            })
        };
        column![picture, sound].spacing(4).into()
    }

    /// Sets where the step `id`'s picture comes from. Another picture is
    /// the one picked; with none picked, nothing changes and the step
    /// says how.
    pub(crate) fn set_picture(&mut self, id: StepId, choice: PictureChoice) {
        let picture = match choice {
            PictureChoice::Own => Picture::Own,
            PictureChoice::Keep => Picture::Keep,
            PictureChoice::Black => Picture::Black,
            PictureChoice::File(_) => match self.picked_picture() {
                Some(file) => Picture::File((*file).clone()),
                None => {
                    self.motion
                        .refuse("Pick a picture in search or the library first");
                    return;
                }
            },
        };
        self.remember(None);
        self.engine.0.edit_plan(|plan| {
            if let Some(step) = plan.get_mut(id) {
                step.picture = picture;
            }
        });
    }

    /// Sets where the step `id`'s sound comes from: its own, the sound
    /// picked, or none.
    pub(crate) fn set_sound(&mut self, id: StepId, choice: SoundChoice) {
        let picked = self.picked_sound();
        if matches!(choice, SoundChoice::File(_)) && picked.is_none() {
            self.motion
                .refuse("Pick a sound or a video in search or the library first");
            return;
        }
        self.remember(None);
        self.engine.0.edit_plan(|plan| {
            let Some(step) = plan.get_mut(id) else {
                return;
            };
            match choice {
                SoundChoice::Own => {
                    step.sound = None;
                    step.silent = false;
                }
                SoundChoice::None => {
                    step.sound = None;
                    step.silent = true;
                }
                SoundChoice::File(_) => {
                    if let Some(file) = picked {
                        step.sound = Some(StepSound::new((*file).clone()));
                        step.silent = false;
                    }
                }
            }
        });
    }
}

/// One line of the card: its icon, and what it is set to, to pick
/// another.
fn chooser<'a, T>(
    p: Palette,
    icon: Icon,
    choices: Vec<T>,
    now: T,
    on: impl Fn(T) -> Message + 'a,
) -> Element<'a, Message>
where
    T: fmt::Display + Clone + PartialEq + 'a,
{
    let ground = iced::Border {
        radius: 8.0.into(),
        ..iced::Border::default()
    };
    let list = pick_list(choices, Some(now), on)
        .text_size(META)
        .padding([2, 6])
        .style(move |_, _| pick_list::Style {
            text_color: p.on_surface,
            placeholder_color: p.on_surface_variant,
            handle_color: p.on_surface_variant,
            background: p.surface_container_highest.into(),
            border: ground,
        })
        .menu_style(move |_| iced::widget::overlay::menu::Style {
            background: p.surface_container_highest.into(),
            border: ground,
            text_color: p.on_surface,
            selected_text_color: p.on_primary,
            selected_background: p.primary.into(),
            shadow: iced::Shadow::default(),
        });
    row![look::icon(icon, 12.0, p.on_surface_variant), list]
        .spacing(5)
        .align_y(Alignment::Center)
        .into()
}

/// What a row that mixes says under its name: where its picture and its
/// sound come from, when not from what it is.
pub fn mix_lines(step: &Step) -> Vec<(Icon, String)> {
    let Action::Show { .. } = &step.action else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    match &step.picture {
        Picture::Own => {}
        Picture::File(file) => lines.push((Icon::Image, format!("Shows {}", file.title))),
        Picture::Keep => lines.push((Icon::Image, "Keeps the picture that was on".to_string())),
        Picture::Black => lines.push((Icon::Image, "Shows nothing".to_string())),
    }
    if step.silent && step.sound.is_none() {
        lines.push((Icon::Music, "Silent".to_string()));
    }
    lines
}
