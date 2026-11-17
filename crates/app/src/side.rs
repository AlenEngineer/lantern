//! The panel beside the preview while planning: one place, three lenses.
//!
//! **Add** is the library, to build the plan from. **Look** is how what
//! is selected looks on the projector: each value says which level it
//! comes from (all plans, the plan, a section around it) and can be set
//! here instead. **Check** is what would go wrong, seen early: what isn't
//! ready, each section's time, where sound and the stage go.

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input, tooltip};
use iced::{Alignment, Element, Fill, Length};
use lantern_engine::{Look, LookChange, Phase, Plan, Rgb, Step, StepId};

use crate::look::{self, BODY, BOLD, Icon, META, Tone};
use crate::outputs::Place;
use crate::ready::Level;
use crate::widgets::{self, Filled, hinted};
use crate::{App, Message, length};

/// What the side panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lens {
    #[default]
    Add,
    Look,
    Check,
}

/// One thing a look says, as the Look lens edits it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Background,
    /// The picture over the background colour.
    Picture,
    Text,
    Font,
    Largest,
    Smallest,
    Dim,
    Reference,
}

impl Field {
    pub const ALL: [Field; 8] = [
        Field::Background,
        Field::Picture,
        Field::Text,
        Field::Font,
        Field::Largest,
        Field::Smallest,
        Field::Dim,
        Field::Reference,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Field::Background => "Background",
            Field::Picture => "Picture",
            Field::Text => "Text colour",
            Field::Font => "Font",
            Field::Largest => "Largest text",
            Field::Smallest => "Smallest text",
            Field::Dim => "Picture dimmed",
            Field::Reference => "Reference line",
        }
    }

    /// What `look` says of it, as its field shows it.
    pub(crate) fn of(self, look: &Look) -> String {
        match self {
            Field::Background => look.background.to_string(),
            Field::Picture => look
                .image
                .as_ref()
                .map(|image| image.display().to_string())
                .unwrap_or_default(),
            Field::Text => look.text.to_string(),
            Field::Font => look.font.clone().unwrap_or_default(),
            Field::Largest => look.max_size.to_string(),
            Field::Smallest => look.min_size.to_string(),
            Field::Dim => look.dim.to_string(),
            Field::Reference => match look.no_reference {
                true => "hidden".into(),
                false => "shown".into(),
            },
        }
    }

    /// Its colour, for the swatch beside a colour's field.
    pub(crate) fn colour(self, look: &Look) -> Option<Rgb> {
        match self {
            Field::Background => Some(look.background),
            Field::Text => Some(look.text),
            _ => None,
        }
    }

    /// Whether `change` says anything of it.
    fn set_in(self, change: &LookChange) -> bool {
        match self {
            Field::Background => change.background.is_some(),
            Field::Picture => change.image.is_some() || change.no_image,
            Field::Text => change.text.is_some(),
            Field::Font => change.font.is_some(),
            Field::Largest => change.max_size.is_some(),
            Field::Smallest => change.min_size.is_some(),
            Field::Dim => change.dim.is_some(),
            Field::Reference => change.reference.is_some(),
        }
    }

    /// Puts what was typed into `change`. `false` when it can't be read
    /// (yet): then nothing is changed.
    fn put(self, change: &mut LookChange, typed: &str) -> bool {
        let typed = typed.trim();
        let size = || {
            typed
                .parse::<u16>()
                .ok()
                .filter(|size| (1..=400).contains(size))
        };
        match self {
            Field::Background => typed
                .parse()
                .map(|colour| change.background = Some(colour))
                .is_ok(),
            // Empty takes the picture away; the caller has seen that
            // anything else is a file.
            Field::Picture => {
                change.no_image = typed.is_empty();
                change.image = (!typed.is_empty()).then(|| typed.into());
                true
            }
            Field::Text => typed
                .parse()
                .map(|colour| change.text = Some(colour))
                .is_ok(),
            Field::Font => {
                if typed.is_empty() {
                    return false;
                }
                change.font = Some(typed.to_string());
                true
            }
            Field::Largest => size().map(|size| change.max_size = Some(size)).is_some(),
            Field::Smallest => size().map(|size| change.min_size = Some(size)).is_some(),
            Field::Dim => typed
                .parse::<u8>()
                .ok()
                .filter(|dim| *dim <= 100)
                .map(|dim| change.dim = Some(dim))
                .is_some(),
            Field::Reference => {
                change.reference = Some(typed == "shown");
                true
            }
        }
    }

    /// Takes it out of `change`: the level above says again.
    fn clear(self, change: &mut LookChange) {
        match self {
            Field::Background => change.background = None,
            Field::Picture => {
                change.image = None;
                change.no_image = false;
            }
            Field::Text => change.text = None,
            Field::Font => change.font = None,
            Field::Largest => change.max_size = None,
            Field::Smallest => change.min_size = None,
            Field::Dim => change.dim = None,
            Field::Reference => change.reference = None,
        }
    }

    /// Makes `to` say of it what `from` says.
    fn copy(self, to: &mut Look, from: &Look) {
        match self {
            Field::Background => to.background = from.background,
            Field::Picture => to.image.clone_from(&from.image),
            Field::Text => to.text = from.text,
            Field::Font => to.font.clone_from(&from.font),
            Field::Largest => to.max_size = from.max_size,
            Field::Smallest => to.min_size = from.min_size,
            Field::Dim => to.dim = from.dim,
            Field::Reference => to.no_reference = from.no_reference,
        }
    }
}

/// A bar on a card: how full, over a track that shows on the card's
/// ground.
fn track<'a>(p: crate::palette::Palette, done: f32, colour: iced::Color) -> Element<'a, Message> {
    widgets::bar(
        [
            (done, Filled::Solid(colour)),
            (1.0 - done, Filled::Solid(p.outline_variant)),
        ],
        3.0,
    )
    .into()
}

/// Where a value in the Look lens comes from.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum From {
    Here,
    Section(String),
    Plan,
    Usual,
}

/// Where `field` of the look of `target` (a step or section, or the plan
/// as `None`) comes from: itself, the nearest section around it that
/// says, the plan's own look where that differs from the `usual` one, or
/// the usual one.
pub(crate) fn origin(plan: &Plan, usual: &Look, target: Option<StepId>, field: Field) -> From {
    let from_plan = || match &plan.look {
        Some(own) if field.of(own) != field.of(usual) => From::Plan,
        _ => From::Usual,
    };
    let Some(at) = target.and_then(|id| plan.position(id)) else {
        return match from_plan() {
            From::Plan => From::Here,
            other => other,
        };
    };
    let says = |step: &Step| {
        step.look
            .as_ref()
            .is_some_and(|change| field.set_in(change))
    };
    if says(&plan.steps()[at]) {
        return From::Here;
    }
    plan.within(at)
        .into_iter()
        .rev()
        .filter_map(|id| plan.get(id))
        .find(|section| says(section))
        .map_or_else(from_plan, |section| From::Section(section.label.clone()))
}

/// What sets `field` of a look: a box to type in, with the colour beside
/// one that is a colour; for the reference line, a pill that shows or
/// hides it. `typed` is what the box holds.
pub(crate) fn look_control<'a>(
    p: crate::palette::Palette,
    field: Field,
    look: &Look,
    typed: &str,
) -> Element<'a, Message> {
    if field == Field::Reference {
        let shown = !look.no_reference;
        return button(text(if shown { "Shown" } else { "Hidden" }).size(META))
            .padding([4, 10])
            .on_press(Message::LookEdit(
                field,
                if shown { "hidden" } else { "shown" }.to_string(),
            ))
            .style(look::pill(
                p,
                if shown { Tone::Selected } else { Tone::Plain },
            ))
            .into();
    }
    let input = text_input(field.name(), typed)
        .on_input(move |typed| Message::LookEdit(field, typed))
        .size(META)
        .padding([4, 8])
        .style(look::input(p));
    match field.colour(look) {
        Some(Rgb([r, g, b])) => row![
            widgets::swatch(iced::Color::from_rgb8(r, g, b), 16.0, 4.0, p.outline),
            input
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into(),
        None => input.into(),
    }
}

/// A value set at this level says so, and a press on it gives the value
/// back to the level above.
pub(crate) fn set_here<'a>(
    p: crate::palette::Palette,
    field: Field,
    size: f32,
    about: &str,
) -> Element<'a, Message> {
    hinted(
        p,
        button(text("set here ✕").size(size).font(BOLD).color(p.primary))
            .padding([3, 8])
            .on_press(Message::LookReset(field))
            .style(look::pill(p, Tone::Selected)),
        about.to_string(),
        tooltip::Position::Left,
    )
}

impl App {
    /// What the box of `field` shows for the look of `target`: what is
    /// being typed there, or else what `look` says.
    pub(crate) fn look_shown(&self, target: Option<StepId>, field: Field, look: &Look) -> String {
        match &self.look_typed {
            Some((of, typing, typed)) if *of == target && *typing == field => typed.clone(),
            _ => field.of(look),
        }
    }

    /// Whose look the Look lens edits: the selected step or section, or
    /// with nothing selected the plan (`None`).
    fn look_target(&self) -> Option<StepId> {
        // The plan's own panel edits the plan's look, whatever is selected.
        self.selected
            .filter(|_| !self.plan_panel)
            .filter(|id| self.engine.0.plan().get(*id).is_some())
    }

    /// What was typed into `field` of the Look lens: kept as typed, and
    /// put into the target's look as soon as it reads as a value.
    pub(crate) fn look_edit(&mut self, field: Field, typed: String) {
        let target = self.look_target();
        let usual = self.config.stage.clone();
        let mut change = LookChange::default();
        // A picture is kept as typed until it is a file there is; then
        // it is held whole (`~` and all), as the stage reads it.
        let (typed, value) = match field {
            Field::Picture if !typed.trim().is_empty() => {
                let file = self.config_status.path();
                let dir = file.parent().unwrap_or(std::path::Path::new("."));
                let path = crate::config::resolve(
                    std::path::Path::new(typed.trim()),
                    dir,
                    self.home.as_deref(),
                );
                let value = path.is_file().then(|| path.display().to_string());
                (typed, value)
            }
            _ => (typed.clone(), Some(typed)),
        };
        if let Some(typed) = value.filter(|value| field.put(&mut change, value)) {
            self.remember(None);
            self.engine.0.edit_plan(|plan| match target {
                Some(id) => {
                    if let Some(step) = plan.get_mut(id) {
                        field.put(step.look.get_or_insert_with(LookChange::default), &typed);
                    }
                }
                // The plan's own look is whole: it starts as the usual
                // one, and is dropped when it says nothing else.
                None => {
                    let own = plan.look.get_or_insert_with(|| usual.clone());
                    change.apply(own);
                    if *own == usual {
                        plan.look = None;
                    }
                }
            });
        }
        self.look_typed = Some((target, field, typed));
    }

    /// `field` goes back to what the level above says.
    pub(crate) fn look_reset(&mut self, field: Field) {
        let target = self.look_target();
        let usual = self.config.stage.clone();
        self.look_typed = None;
        self.remember(None);
        self.engine.0.edit_plan(|plan| match target {
            Some(id) => {
                if let Some(step) = plan.get_mut(id) {
                    if let Some(change) = &mut step.look {
                        field.clear(change);
                    }
                    if step.look.as_ref().is_some_and(LookChange::is_empty) {
                        step.look = None;
                    }
                }
            }
            None => {
                if let Some(own) = &mut plan.look {
                    field.copy(own, &usual);
                    if *own == usual {
                        plan.look = None;
                    }
                }
            }
        });
    }

    /// The panel: its three lenses, and what the one in use shows.
    pub(crate) fn side_panel(&self) -> Element<'_, Message> {
        // A fill under way has the panel to itself.
        if let Some(fill) = self.fill_card() {
            return fill;
        }
        // So do the open plan's own settings.
        if let Some(plan) = self.plan_card() {
            return plan;
        }
        let p = self.palette;
        let plan = self.engine.0.plan();
        let waiting = self.issues(&plan).len();
        let lens = |lens: Lens, icon: Icon, about: &str, count: usize| {
            let on = self.lens == lens;
            let ink = widgets::place_ink(p, on);
            let mut content = row![look::icon(icon, 16.0, ink)]
                .spacing(6)
                .align_y(Alignment::Center);
            if count > 0 {
                content = content.push(text(count.to_string()).size(META).font(BOLD).color(ink));
            }
            widgets::place(p, content, on, 6, about, Message::Lens(lens))
        };
        // One switch with three places, not three buttons.
        let lenses = widgets::places(
            p,
            [
                lens(Lens::Add, Icon::Plus, "Add: the library", 0),
                lens(
                    Lens::Look,
                    Icon::Tune,
                    "Look: how what is selected looks",
                    0,
                ),
                lens(Lens::Check, Icon::Check, "Check: what isn't ready", waiting),
            ],
            16.0,
        );
        let body: Element<'_, Message> = match self.lens {
            Lens::Add => self.shelf_pane(),
            Lens::Look => scrollable(self.look_lens(&plan)).height(Fill).into(),
            Lens::Check => scrollable(self.check_lens(&plan)).height(Fill).into(),
        };
        column![lenses, body].spacing(12).height(Fill).into()
    }

    fn look_lens<'a>(&self, plan: &Plan) -> Element<'a, Message> {
        let p = self.palette;
        let target = self.look_target();
        let at = target.and_then(|id| plan.position(id));
        let look = match at {
            Some(at) => plan.look_at(at, &self.config.stage),
            None => plan
                .look
                .clone()
                .unwrap_or_else(|| self.config.stage.clone()),
        };
        // The way down to it: each level is a click away.
        let crumb = |label: String, on: bool, message: Option<Message>| {
            // A long name is cut: a crumb is one line.
            let label = match label.chars().count() > 26 {
                true => format!("{}…", label.chars().take(25).collect::<String>()),
                false => label,
            };
            button(
                text(label)
                    .size(META)
                    .font(if on { BOLD } else { look::MEDIUM }),
            )
            .padding([4, 10])
            .on_press_maybe(message)
            .style(look::pill(p, if on { Tone::Selected } else { Tone::Plain }))
        };
        let mut crumbs = row![crumb(
            "All plans".into(),
            false,
            Some(Message::Key {
                op: crate::keys::Op::Settings,
                only_if_empty: false,
            }),
        )]
        .spacing(4);
        crumbs = crumbs.push(crumb(
            plan.title.clone(),
            target.is_none(),
            Some(Message::Unselect),
        ));
        if let Some(at) = at {
            for id in plan.within(at) {
                if let Some(section) = plan.get(id) {
                    crumbs = crumbs.push(crumb(
                        section.label.clone(),
                        false,
                        Some(Message::Select(id)),
                    ));
                }
            }
            crumbs = crumbs.push(crumb(plan.steps()[at].label.clone(), true, None));
        }
        let holds = match at.map(|at| &plan.steps()[at]) {
            Some(step) if step.is_section() => {
                "Holds for everything in this section, unless a step sets its own."
            }
            Some(_) => "For this step only.",
            None => "Holds for the whole plan, unless a section or a step sets its own.",
        };
        let mut fields = column![].spacing(8);
        for field in Field::ALL {
            let from = origin(plan, &self.config.stage, target, field);
            let here = matches!(from, From::Here);
            let typed = self.look_shown(target, field, &look);
            let control = look_control(p, field, &look, &typed);
            let origin: Element<'a, Message> = match from {
                From::Here => set_here(p, field, META, "Back to what the level above gives"),
                From::Section(name) => text(format!("from {name}"))
                    .size(META)
                    .color(p.on_surface_variant)
                    .into(),
                From::Plan => text("from the plan")
                    .size(META)
                    .color(p.on_surface_variant)
                    .into(),
                From::Usual => text("from all plans")
                    .size(META)
                    .color(p.on_surface_variant)
                    .into(),
            };
            fields = fields.push(
                column![
                    row![
                        text(field.name())
                            .size(META)
                            .font(if here { BOLD } else { look::MEDIUM })
                            .color(if here {
                                p.primary
                            } else {
                                p.on_surface_variant
                            }),
                        Space::new().width(Fill),
                        origin,
                    ]
                    .align_y(Alignment::Center),
                    control,
                ]
                .spacing(4),
            );
        }
        column![
            crumbs.wrap(),
            text(holds).size(META).color(p.on_surface_variant),
            fields,
        ]
        .spacing(12)
        .into()
    }

    fn check_lens<'a>(&self, plan: &Plan) -> Element<'a, Message> {
        let p = self.palette;
        let card = |content: Element<'a, Message>| {
            container(content)
                .width(Fill)
                .padding([8, 10])
                .style(move |_| container::Style {
                    background: Some(p.surface_container_highest.into()),
                    border: iced::Border {
                        radius: 12.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                })
        };
        let mut body = column![].spacing(10);
        // What isn't ready, each a click from its step.
        let issues = self.issues(plan);
        if issues.is_empty() {
            body = body.push(card(
                row![
                    look::icon(Icon::Check, 16.0, p.primary),
                    text("Every step is ready").size(BODY).color(p.on_surface)
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
            ));
        }
        for issue in issues {
            let colour = match issue.level {
                Level::Problem => p.error,
                Level::Waiting => p.tertiary,
            };
            let step = plan.get(issue.step);
            let label = step.map_or_else(String::new, |step| step.label.clone());
            // What is coming in has a ring for how far; what is wrong
            // says what, under its name.
            let coming = step.and_then(|step| match &step.action {
                lantern_engine::Action::Show { resource } => self.download_part(resource),
                _ => None,
            });
            let name = container(text(label).size(BODY).wrapping(text::Wrapping::None))
                .width(Fill)
                .clip(true);
            let row: Element<'a, Message> = match coming {
                Some(part) => row![
                    look::ring(part, 18.0, colour),
                    name,
                    text(format!("{:.0}%", part * 100.0))
                        .size(META)
                        .color(colour),
                ]
                .spacing(10)
                .align_y(Alignment::Center)
                .into(),
                None => column![name, text(issue.what).size(META).color(colour)]
                    .spacing(2)
                    .into(),
            };
            body = body.push(
                button(row)
                    .width(Fill)
                    .padding([8, 10])
                    .on_press(Message::Select(issue.step))
                    .style(look::row_filled(p, p.surface_container_highest)),
            );
        }
        // Each of the plan's own sections against its time.
        let timing = self.timing();
        let mut times = column![].spacing(8);
        let mut any = false;
        for section in &timing.sections {
            if !plan
                .get(section.id)
                .is_some_and(|step| step.depth() == Some(0))
            {
                continue;
            }
            any = true;
            let planned = match section.phase {
                Phase::Coming { planned } => planned,
                Phase::Running { expected, .. } => expected,
                Phase::Done { took } => took.unwrap_or_default(),
            };
            let (words, full, colour) = match section.budget {
                Some(budget) if !budget.is_zero() => (
                    format!(
                        "{}/{}",
                        (planned.as_secs() + 30) / 60,
                        (budget.as_secs() + 30) / 60
                    ),
                    planned.as_secs_f32() / budget.as_secs_f32(),
                    if planned > budget {
                        p.tertiary
                    } else {
                        p.primary
                    },
                ),
                _ => (length::clock(planned), 1.0, p.primary),
            };
            times = times.push(
                column![
                    row![
                        look::icon(Icon::Clock, 15.0, colour),
                        container(
                            text(section.title.clone())
                                .size(BODY)
                                .wrapping(text::Wrapping::None)
                        )
                        .width(Fill)
                        .clip(true),
                        text(words).size(META).color(colour),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                    track(p, full, colour),
                ]
                .spacing(5),
            );
        }
        if any {
            body = body.push(card(times.into()));
        }
        // Where sound and the stage go, as they stood when this lens was
        // opened.
        let sound: Element<'a, Message> = match &self.sound_now {
            Some(sound) => {
                let off = sound.muted || sound.volume < 0.01;
                let colour = if off { p.error } else { p.primary };
                row![
                    look::icon(Icon::Sound, 16.0, colour),
                    container(
                        text(if sound.output.is_empty() {
                            "Sound".to_string()
                        } else {
                            sound.output.clone()
                        })
                        .size(BODY)
                        .wrapping(text::Wrapping::None)
                    )
                    .width(Fill)
                    .clip(true),
                    container(track(
                        p,
                        if sound.muted { 0.0 } else { sound.volume },
                        colour
                    ))
                    .width(44),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .into()
            }
            None => row![
                look::icon(Icon::Sound, 16.0, p.on_surface_variant),
                text("?").size(BODY).color(p.on_surface_variant)
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
        };
        let screen = match &self.place {
            Some(Place::Preferred(name) | Place::Remembered(name) | Place::Spare(name)) => {
                (name.clone(), p.on_surface)
            }
            Some(Place::Nowhere) | None => ("No projector".to_string(), p.on_surface_variant),
        };
        body = body.push(card(
            column![
                sound,
                row![
                    look::icon(
                        if self
                            .place
                            .as_ref()
                            .is_some_and(|place| *place != Place::Nowhere)
                        {
                            Icon::Screen
                        } else {
                            Icon::ScreenOff
                        },
                        16.0,
                        screen.1
                    ),
                    text(screen.0).size(BODY).color(screen.1)
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8)
            .into(),
        ));
        body.width(Length::Fill).into()
    }
}

#[cfg(test)]
mod tests {
    use lantern_engine::Action;

    use super::*;

    #[test]
    fn a_value_says_which_level_it_comes_from() {
        let usual = Look::default();
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        let praise = plan.section("Praise", None);
        let song = plan.push("Song", Action::Blank);
        assert!(plan.nest(praise, true));
        let field = Field::Background;
        // Nothing says: the look for all plans.
        assert_eq!(origin(&plan, &usual, Some(song), field), From::Usual);
        assert_eq!(origin(&plan, &usual, None, field), From::Usual);
        // The plan's own look, where it differs.
        plan.look = Some(Look {
            background: Rgb([1, 2, 3]),
            ..Look::default()
        });
        assert_eq!(origin(&plan, &usual, Some(song), field), From::Plan);
        assert_eq!(origin(&plan, &usual, None, field), From::Here);
        assert_eq!(origin(&plan, &usual, Some(song), Field::Font), From::Usual);
        // The outer section, then the nearer one, then the step itself.
        let mut change = LookChange::default();
        assert!(field.put(&mut change, "#102030"));
        plan.get_mut(music).unwrap().look = Some(change.clone());
        assert_eq!(
            origin(&plan, &usual, Some(song), field),
            From::Section("Music".into())
        );
        plan.get_mut(praise).unwrap().look = Some(change.clone());
        assert_eq!(
            origin(&plan, &usual, Some(song), field),
            From::Section("Praise".into())
        );
        assert_eq!(origin(&plan, &usual, Some(praise), field), From::Here);
        plan.get_mut(song).unwrap().look = Some(change);
        assert_eq!(origin(&plan, &usual, Some(song), field), From::Here);
    }

    #[test]
    fn a_field_takes_only_what_reads_as_its_value() {
        let mut change = LookChange::default();
        assert!(!Field::Background.put(&mut change, "#1c"));
        assert!(Field::Background.put(&mut change, "#1c2b44"));
        assert!(!Field::Largest.put(&mut change, "0"));
        assert!(Field::Largest.put(&mut change, "72"));
        assert!(!Field::Dim.put(&mut change, "140"));
        assert!(Field::Reference.put(&mut change, "hidden"));
        assert_eq!(change.max_size, Some(72));
        assert_eq!(change.reference, Some(false));
        Field::Largest.clear(&mut change);
        assert!(!Field::Largest.set_in(&change));
    }

    #[test]
    fn a_picture_is_set_taken_away_and_given_back() {
        let mut change = LookChange::default();
        assert!(!Field::Picture.set_in(&change));
        assert!(Field::Picture.put(&mut change, "/media/night.jpg"));
        assert_eq!(change.image, Some("/media/night.jpg".into()));
        assert!(!change.no_image);
        // Emptied, the level says there is no picture: that is said too.
        assert!(Field::Picture.put(&mut change, " "));
        assert!(change.image.is_none() && change.no_image);
        assert!(Field::Picture.set_in(&change));
        Field::Picture.clear(&mut change);
        assert!(change.is_empty(), "back to what the level above gives");
    }
}
