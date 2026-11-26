//! Where a picked step's picture and sound come from, in its card: its
//! own, another file (the one picked in search or the library), what was
//! on kept, or nothing. And the lines under a row that says so, on the
//! rows that mix. And how it lays out what it shows with a second
//! thing. And where the card is open: under the preview while planning,
//! under its row in a run ([`App::card_open`]).

use std::fmt;
use std::sync::Arc;

use iced::widget::{button, column, container, pick_list, row, text, text_input};
use iced::{Alignment, Element, Fill, Length};
use lantern_engine::{
    Action, FadeTime, Kind, Layout, Location, Meets, Picture, Plan, Resource, ResourceId, SourceId,
    Step, StepId, StepSound,
};

use crate::FadeEnd;
use crate::look::{self, BOLD, Icon, META, Tone};
use crate::mode::Mode;
use crate::palette::Palette;
use crate::widgets::caption;
use crate::{App, Message};

/// Where a step's look and timing are open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardAt {
    /// In a card under its row.
    Row,
    /// Under the preview, a group of their own.
    Preview,
}

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

/// How far a step's sound keeps going: its own step alone, or under the
/// steps after it, up to one.
#[derive(Debug, Clone, PartialEq)]
pub struct Until {
    pub to: Option<StepId>,
    label: String,
}

impl fmt::Display for Until {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

/// What a sound that keeps going does under a step with a sound of its
/// own, in words.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeetsChoice(pub Meets);

impl fmt::Display for MeetsChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            Meets::Fade => "fades out under a sound",
            Meets::Cut => "stops at a sound",
            Meets::Under => "is a background track",
        })
    }
}

/// How far `step`'s sound can keep going: its step alone, or to any of
/// the steps after it.
fn untils(plan: &Plan, step: &Step) -> Vec<Until> {
    let mut choices = vec![Until {
        to: None,
        label: "for this step".to_string(),
    }];
    if let Some(at) = plan.position(step.id) {
        choices.extend(
            plan.steps()[at + 1..]
                .iter()
                .filter(|later| !later.is_section())
                .map(|later| Until {
                    to: Some(later.id),
                    label: format!("on to {}", later.label),
                }),
        );
    }
    choices
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
    /// The step whose look and timing are open, and where. While
    /// planning, the one whose Look & timing was pressed, under the
    /// preview; in a run unlocked, the selected step while it is still
    /// ahead, under its row. Only a step that shows something has them.
    pub(crate) fn card_open(&self) -> Option<(StepId, CardAt)> {
        if !self.can_edit() {
            return None;
        }
        let (id, at) = match self.mode {
            Mode::Plan => (self.look_and_timing?, CardAt::Preview),
            Mode::Run => (self.selected?, CardAt::Row),
            Mode::Live => return None,
        };
        let plan = self.engine.0.plan();
        let shows = plan
            .get(id)
            .is_some_and(|step| matches!(step.action, Action::Show { .. }));
        // In a run, only a step still ahead can change.
        let changes = self.mode != Mode::Run || self.ahead(&plan, id);
        (shows && changes).then_some((id, at))
    }

    /// `step`'s look and timing as they show `at`: where its picture and
    /// its sound come from, how it is laid out, and how it fades. Under
    /// its row they are a card. Under the preview they are a group of
    /// their own, set apart by space and their name, each chooser as wide
    /// as the preview.
    pub(crate) fn look_and_timing_card<'a>(&self, step: &Step, at: CardAt) -> Element<'a, Message> {
        let p = self.palette;
        let width = match at {
            CardAt::Row => Length::Shrink,
            CardAt::Preview => Length::Fill,
        };
        let lines = column![
            self.sources_card(p, step, width),
            crate::plan_pane::fade_steppers(p, step, width),
        ]
        .spacing(6);
        match at {
            CardAt::Row => container(
                container(lines)
                    .padding([4, 5])
                    .style(look::card(p, 0.0, 10.0)),
            )
            .padding(iced::Padding::default().top(6.0))
            .into(),
            CardAt::Preview => column![caption(p, "LOOK & TIMING"), lines]
                .spacing(8)
                .padding(iced::Padding::default().top(6.0))
                .into(),
        }
    }

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

    /// The lines of a picked step's card that set where its picture and
    /// its sound come from, each chooser `width` wide.
    pub(crate) fn sources_card<'a>(
        &self,
        p: Palette,
        step: &Step,
        width: Length,
    ) -> Element<'a, Message> {
        let id = step.id;
        let picture = {
            let mut choices = vec![PictureChoice::Own];
            choices.push(PictureChoice::File(
                self.picked_picture().map(|file| file.title.clone()),
            ));
            choices.extend([PictureChoice::Keep, PictureChoice::Black]);
            chooser(
                p,
                Icon::Image,
                choices,
                picture_now(step),
                width,
                move |choice| Message::StepPicture(id, choice),
            )
        };
        let sound = {
            let choices = vec![
                SoundChoice::Own,
                SoundChoice::File(self.picked_sound().map(|file| file.title.clone())),
                SoundChoice::None,
            ];
            chooser(
                p,
                Icon::Music,
                choices,
                sound_now(step),
                width,
                move |choice| Message::StepSoundFrom(id, choice),
            )
        };
        let mut card = column![picture, self.layout_card(p, step, width), sound].spacing(4);
        // A sound of its own can keep going under the steps after it.
        if let Some(set) = &step.sound {
            let plan = self.engine.0.plan();
            let choices = untils(&plan, step);
            let now = choices
                .iter()
                .find(|choice| choice.to == set.until)
                .cloned()
                .unwrap_or_else(|| choices[0].clone());
            card = card.push(chooser(
                p,
                Icon::Joined,
                choices,
                now,
                width,
                move |choice: Until| Message::StepSoundUntil(id, choice.to),
            ));
            if set.until.is_some() {
                card = card.push(chooser(
                    p,
                    Icon::Sound,
                    vec![
                        MeetsChoice(Meets::Fade),
                        MeetsChoice(Meets::Cut),
                        MeetsChoice(Meets::Under),
                    ],
                    MeetsChoice(set.meets),
                    width,
                    move |choice: MeetsChoice| Message::StepSoundMeets(id, choice.0),
                ));
            }
        }
        card.into()
    }

    /// The picked file, when it can be laid out with what a step shows:
    /// a picture, a page, or a text. Never a video.
    fn picked_second(&self) -> Option<Arc<Resource>> {
        self.picked_file()
            .filter(|file| matches!(file.kind, Kind::Image | Kind::Document | Kind::Text))
    }

    /// The card's lines that lay out what the step shows with a second
    /// thing: which layout (its chooser `width` wide), what the second is,
    /// and a quote typed in.
    fn layout_card<'a>(&self, p: Palette, step: &Step, width: Length) -> Element<'a, Message> {
        let id = step.id;
        let now = match &step.layout {
            Layout::One => LayoutChoice::Alone,
            Layout::Beside(_) => LayoutChoice::Beside,
            Layout::LowerThird(_) => LayoutChoice::LowerThird,
        };
        let mut lines = column![chooser(
            p,
            Icon::Screen,
            vec![
                LayoutChoice::Alone,
                LayoutChoice::Beside,
                LayoutChoice::LowerThird
            ],
            now,
            width,
            move |choice| Message::StepLayout(id, choice),
        )]
        .spacing(4);
        let Some(second) = step.layout.second() else {
            return lines.into();
        };
        let (icon, named) = match (&second.kind, second.title.trim()) {
            (Kind::Text, "") => (Icon::Pencil, "nothing yet".to_string()),
            (Kind::Text, title) => (Icon::Pencil, title.to_string()),
            (_, title) => (Icon::Image, title.to_string()),
        };
        let mut said = row![
            look::icon(icon, 12.0, p.on_surface_variant),
            iced::widget::container(
                crate::widgets::single_line(named)
                    .size(META)
                    .color(p.on_surface_variant),
            )
            .width(Fill),
        ]
        .spacing(5)
        .align_y(Alignment::Center);
        if self
            .picked_second()
            .is_some_and(|picked| picked.id != second.id)
        {
            said = said.push(
                button(text("Use picked").size(META).font(BOLD).color(p.on_surface))
                    .padding([1, 6])
                    .on_press(Message::LayoutPicked(id))
                    .style(look::pill(p, Tone::Plain)),
            );
        }
        let typed = match &self.layout_typed {
            Some((at, typed)) if *at == id => typed.clone(),
            _ => String::new(),
        };
        lines = lines.push(said).push(
            text_input("type a quote", &typed)
                .on_input(move |typed| Message::LayoutTyping(id, typed))
                .on_submit(Message::LayoutTyped(id))
                .size(META)
                .padding([2, 6]),
        );
        lines.into()
    }

    /// Lays out what the step `id` shows as `choice` says. The second
    /// thing stays when one was; else it is the picture or text picked,
    /// else a text still to type.
    pub(crate) fn set_layout(&mut self, id: StepId, choice: LayoutChoice) {
        let picked = self.picked_second();
        self.remember(None);
        self.engine.0.edit_plan(|plan| {
            let Some(step) = plan.get_mut(id) else {
                return;
            };
            let second = step
                .layout
                .second()
                .cloned()
                .or_else(|| picked.map(|file| (*file).clone()))
                .unwrap_or_else(|| typed(id, ""));
            step.layout = match choice {
                LayoutChoice::Alone => Layout::One,
                LayoutChoice::Beside => Layout::Beside(second),
                LayoutChoice::LowerThird => Layout::LowerThird(second),
            };
        });
    }

    /// Puts `second` in the step `id`'s layout, as it is laid out.
    pub(crate) fn set_second(&mut self, id: StepId, second: Resource) {
        self.remember(None);
        self.engine.0.edit_plan(|plan| {
            if let Some(step) = plan.get_mut(id) {
                step.layout = match &step.layout {
                    Layout::One => Layout::One,
                    Layout::Beside(_) => Layout::Beside(second),
                    Layout::LowerThird(_) => Layout::LowerThird(second),
                };
            }
        });
    }

    /// The picture or text picked, laid out with the step `id`.
    pub(crate) fn second_picked(&mut self, id: StepId) {
        match self.picked_second() {
            Some(file) => self.set_second(id, (*file).clone()),
            None => self.refuse_on(
                id,
                "Pick a picture or a text in search or the library first",
            ),
        }
    }

    /// What was typed into the step `id`'s card, laid out with it.
    pub(crate) fn second_typed(&mut self, id: StepId) {
        let Some((at, words)) = self.layout_typed.take() else {
            return;
        };
        if at == id && !words.trim().is_empty() {
            self.set_second(id, typed(id, words.trim()));
        }
    }

    /// Sets how far the step `id`'s sound keeps going.
    pub(crate) fn set_until(&mut self, id: StepId, until: Option<StepId>) {
        self.remember(None);
        self.engine.0.edit_plan(|plan| {
            if let Some(sound) = plan.get_mut(id).and_then(|step| step.sound.as_mut()) {
                sound.until = until;
            }
        });
    }

    /// Sets what the step `id`'s sound does under a step with a sound of
    /// its own.
    pub(crate) fn set_meets(&mut self, id: StepId, meets: Meets) {
        self.remember(None);
        self.engine.0.edit_plan(|plan| {
            if let Some(sound) = plan.get_mut(id).and_then(|step| step.sound.as_mut()) {
                sound.meets = meets;
            }
        });
    }

    /// Sets how the step `id`'s sound apart fades: with its picture,
    /// or on its own (starting from the picture's fade), one end
    /// longer or shorter.
    pub(crate) fn set_sound_fade(&mut self, id: StepId, change: Option<(FadeEnd, i64)>) {
        self.remember(Some(id));
        self.engine.0.edit_plan(|plan| {
            let Some(step) = plan.get_mut(id) else {
                return;
            };
            let Some((end, by)) = change else {
                step.sound_fade = None;
                return;
            };
            let fade = step.sound_fade.get_or_insert(step.fade);
            let time = match end {
                FadeEnd::In => &mut fade.fade_in,
                FadeEnd::Out => &mut fade.fade_out,
            };
            let now = time.map_or(0, FadeTime::secs);
            *time = FadeTime::from_secs(now.saturating_add_signed(by));
        });
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
                    self.refuse_on(id, "Pick a picture in search or the library first");
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
            self.refuse_on(id, "Pick a sound or a video in search or the library first");
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

/// How a step lays out what it shows, in words.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayoutChoice {
    Alone,
    Beside,
    LowerThird,
}

impl fmt::Display for LayoutChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LayoutChoice::Alone => "alone",
            LayoutChoice::Beside => "side by side",
            LayoutChoice::LowerThird => "lower third",
        })
    }
}

/// A text typed into the step `id`'s card, to lay out with what it
/// shows; named by its first words.
fn typed(id: StepId, words: &str) -> Resource {
    let source = SourceId::new("typed").expect("a valid source id");
    let name: String = words.chars().take(32).collect();
    let name = match words.chars().count() > 32 {
        true => format!("{}…", name.trim_end()),
        false => name,
    };
    Resource::new(
        ResourceId::new(
            &source,
            &format!("{id}-{:x}", {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                words.hash(&mut hasher);
                hasher.finish()
            }),
        ),
        name,
        Kind::Text,
        Location::Inline(words.to_string()),
    )
}

/// One line of the card: its icon, and what it is set to, `width` wide,
/// to pick another.
fn chooser<'a, T>(
    p: Palette,
    icon: Icon,
    choices: Vec<T>,
    now: T,
    width: Length,
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
        .width(width)
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

/// Whether `step` makes a sound of its own: a video or a sound, heard,
/// or a sound set in place of its own.
fn sounds(step: &Step) -> bool {
    match &step.action {
        Action::Show { resource } => {
            step.sound.is_some()
                || (!step.silent && matches!(resource.kind, Kind::Video | Kind::Audio))
        }
        _ => false,
    }
}

/// What a row that mixes says under its name: where its picture and its
/// sound come from, when not from what it is; and the sound an earlier
/// step set that keeps going under it.
pub fn mix_lines(plan: &Plan, step: &Step) -> Vec<(Icon, String)> {
    let mut lines = Vec::new();
    if let Some(at) = plan.position(step.id) {
        for (from, earlier) in plan.steps()[..at].iter().enumerate() {
            let Some(sound) = &earlier.sound else {
                continue;
            };
            let keeps = sound
                .until
                .and_then(|until| plan.position(until))
                .is_some_and(|until| from < at && at <= until);
            if keeps {
                // Under a step with a sound of its own, as it is set to.
                let words = match (sounds(step), sound.meets) {
                    (false, _) => "keeps going",
                    (true, Meets::Under) => "plays lowered here",
                    (true, Meets::Fade) => "fades out here",
                    (true, Meets::Cut) => "stops here",
                };
                lines.push((Icon::Music, format!("{} {words}", sound.resource.title)));
            }
        }
    }
    let Action::Show { .. } = &step.action else {
        return lines;
    };
    match &step.picture {
        Picture::Own => {}
        Picture::File(file) => lines.push((Icon::Image, format!("Shows {}", file.title))),
        Picture::Keep => lines.push((Icon::Image, "Keeps the picture that was on".to_string())),
        Picture::Black => lines.push((Icon::Image, "Shows nothing".to_string())),
    }
    if step.silent && step.sound.is_none() {
        lines.push((Icon::Music, "Silent".to_string()));
    }
    match &step.layout {
        Layout::One => {}
        Layout::Beside(second) => {
            lines.push((Icon::Screen, format!("Beside it: {}", named(second))));
        }
        Layout::LowerThird(second) => {
            lines.push((Icon::Screen, format!("Lower third: {}", named(second))));
        }
    }
    lines
}

/// Whether `step` plays a sound apart from what it shows: a sound set in
/// place of its own, or its own under another picture.
pub fn plays_apart(step: &Step) -> bool {
    step.sound.is_some() || (!step.picture.is_own() && sounds(step))
}

/// How a step's sound apart fades, in words.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SoundFadeChoice {
    WithPicture,
    Own,
}

impl fmt::Display for SoundFadeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SoundFadeChoice::WithPicture => "with the picture",
            SoundFadeChoice::Own => "on its own",
        })
    }
}

/// The card's line that sets whether `step`'s sound apart fades with its
/// picture or on its own, `width` wide.
pub(crate) fn sound_fade_chooser<'a>(
    p: Palette,
    step: &Step,
    width: Length,
) -> Element<'a, Message> {
    let id = step.id;
    let now = match step.sound_fade {
        None => SoundFadeChoice::WithPicture,
        Some(_) => SoundFadeChoice::Own,
    };
    chooser(
        p,
        Icon::Music,
        vec![SoundFadeChoice::WithPicture, SoundFadeChoice::Own],
        now,
        width,
        move |choice| {
            Message::SoundFade(
                id,
                (choice == SoundFadeChoice::Own).then_some((FadeEnd::In, 0)),
            )
        },
    )
}

/// What a second thing laid out with a step is called: a text still to
/// type is nothing yet.
fn named(second: &Resource) -> &str {
    match second.title.trim() {
        "" => "nothing yet",
        title => title,
    }
}
