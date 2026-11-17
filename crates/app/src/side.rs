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

use crate::dashboard::hinted;
use crate::look::{self, BODY, BOLD, Icon, META, Tone};
use crate::outputs::Place;
use crate::ready::Level;
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
    Text,
    Font,
    Largest,
    Smallest,
    Dim,
    Reference,
}

impl Field {
    pub const ALL: [Field; 7] = [
        Field::Background,
        Field::Text,
        Field::Font,
        Field::Largest,
        Field::Smallest,
        Field::Dim,
        Field::Reference,
    ];

    fn name(self) -> &'static str {
        match self {
            Field::Background => "Background",
            Field::Text => "Text colour",
            Field::Font => "Font",
            Field::Largest => "Largest text",
            Field::Smallest => "Smallest text",
            Field::Dim => "Picture dimmed",
            Field::Reference => "Reference line",
        }
    }

    /// What `look` says of it, as its field shows it.
    fn of(self, look: &Look) -> String {
        match self {
            Field::Background => look.background.to_string(),
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
    fn colour(self, look: &Look) -> Option<Rgb> {
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
            Field::Text => to.text = from.text,
            Field::Font => to.font.clone_from(&from.font),
            Field::Largest => to.max_size = from.max_size,
            Field::Smallest => to.min_size = from.min_size,
            Field::Dim => to.dim = from.dim,
            Field::Reference => to.no_reference = from.no_reference,
        }
    }
}

/// Where a value in the Look lens comes from.
#[derive(Debug, PartialEq, Eq)]
enum From {
    Here,
    Section(String),
    Plan,
    Usual,
}

/// Where `field` of the look of `target` (a step or section, or the plan
/// as `None`) comes from: itself, the nearest section around it that
/// says, the plan's own look where that differs from the `usual` one, or
/// the usual one.
fn origin(plan: &Plan, usual: &Look, target: Option<StepId>, field: Field) -> From {
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

impl App {
    /// Whose look the Look lens edits: the selected step or section, or
    /// with nothing selected the plan (`None`).
    fn look_target(&self) -> Option<StepId> {
        self.selected
            .filter(|id| self.engine.0.plan().get(*id).is_some())
    }

    /// What was typed into `field` of the Look lens: kept as typed, and
    /// put into the target's look as soon as it reads as a value.
    pub(crate) fn look_edit(&mut self, field: Field, typed: String) {
        let target = self.look_target();
        let usual = self.config.stage.clone();
        let mut change = LookChange::default();
        if field.put(&mut change, &typed) {
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
        let p = self.palette;
        let plan = self.engine.0.plan();
        let waiting = self.issues(&plan).len();
        let lens = |lens: Lens, icon: Icon, about: &str, count: usize| {
            let on = self.lens == lens;
            let ink = if on { p.on_primary } else { p.on_surface };
            let mut content = row![look::icon(icon, 16.0, ink)]
                .spacing(6)
                .align_y(Alignment::Center);
            if count > 0 {
                content = content.push(text(count.to_string()).size(META).font(BOLD).color(ink));
            }
            hinted(
                p,
                button(container(content).center_x(Fill))
                    .width(Fill)
                    .padding([6, 0])
                    .on_press(Message::Lens(lens))
                    .style(look::pill(p, if on { Tone::Solid } else { Tone::Plain })),
                about.to_string(),
                tooltip::Position::Bottom,
            )
        };
        let lenses = row![
            lens(Lens::Add, Icon::Plus, "Add: the library", 0),
            lens(
                Lens::Look,
                Icon::Tune,
                "Look: how what is selected looks",
                0
            ),
            lens(Lens::Check, Icon::Check, "Check: what isn't ready", waiting),
        ]
        .spacing(4);
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
            let typed = match &self.look_typed {
                Some((of, typing, typed)) if *of == target && *typing == field => typed.clone(),
                _ => field.of(&look),
            };
            let control: Element<'a, Message> = match field {
                Field::Reference => {
                    let shown = !look.no_reference;
                    button(text(if shown { "Shown" } else { "Hidden" }).size(META))
                        .padding([4, 10])
                        .on_press(Message::LookEdit(
                            field,
                            if shown { "hidden" } else { "shown" }.to_string(),
                        ))
                        .style(look::pill(
                            p,
                            if shown { Tone::Selected } else { Tone::Plain },
                        ))
                        .into()
                }
                _ => {
                    let input = text_input(field.name(), &typed)
                        .on_input(move |typed| Message::LookEdit(field, typed))
                        .size(META)
                        .padding([4, 8])
                        .style(look::input(p));
                    match field.colour(&look) {
                        Some(Rgb([r, g, b])) => row![
                            container(Space::new().width(16).height(16)).style(move |_| {
                                container::Style {
                                    background: Some(iced::Color::from_rgb8(r, g, b).into()),
                                    border: iced::Border {
                                        color: p.outline,
                                        width: 1.0,
                                        radius: 4.0.into(),
                                    },
                                    ..container::Style::default()
                                }
                            }),
                            input
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center)
                        .into(),
                        None => input.into(),
                    }
                }
            };
            let origin: Element<'a, Message> = match from {
                From::Here => hinted(
                    p,
                    button(text("set here ✕").size(META).font(BOLD).color(p.primary))
                        .padding([3, 8])
                        .on_press(Message::LookReset(field))
                        .style(look::pill(p, Tone::Selected)),
                    "Back to what the level above gives".to_string(),
                    tooltip::Position::Left,
                ),
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
        let heading = |words: &'static str| {
            text(words)
                .size(META)
                .font(BOLD)
                .color(p.on_surface_variant)
        };
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
            let label = plan
                .get(issue.step)
                .map_or_else(String::new, |step| step.label.clone());
            body = body.push(
                button(
                    column![
                        container(
                            text(label)
                                .size(BODY)
                                .font(BOLD)
                                .wrapping(text::Wrapping::None)
                        )
                        .width(Fill)
                        .clip(true),
                        text(issue.what).size(META).color(colour),
                    ]
                    .spacing(2),
                )
                .width(Fill)
                .padding([8, 10])
                .on_press(Message::Select(issue.step))
                .style(look::row(p, Tone::Selected)),
            );
        }
        // Each of the plan's own sections against its time.
        let timing = self.timing();
        let mut times = column![].spacing(8);
        let mut any = false;
        for section in &timing.sections {
            if plan.get(section.id).and_then(Step::depth) != Some(0) {
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
                    format!("{} / {}", length::clock(planned), length::clock(budget)),
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
                    crate::plan_pane::bar(p, full, colour, 0.0),
                ]
                .spacing(5),
            );
        }
        if any {
            body = body.push(heading("TIME")).push(card(times.into()));
        }
        // Where sound and the stage go, as they stood when this lens was
        // opened.
        let sound: Element<'a, Message> = match &self.sound_now {
            Some(sound) => row![
                look::icon(
                    Icon::Music,
                    16.0,
                    if sound.muted { p.error } else { p.primary }
                ),
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
                text(if sound.muted {
                    "muted".to_string()
                } else {
                    format!("{:.0}%", sound.volume * 100.0)
                })
                .size(META)
                .color(if sound.muted || sound.volume < 0.01 {
                    p.error
                } else {
                    p.on_surface_variant
                }),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
            None => text("Sound: can't tell here")
                .size(META)
                .color(p.on_surface_variant)
                .into(),
        };
        let screen = match &self.place {
            Some(Place::Preferred(name) | Place::Remembered(name) | Place::Spare(name)) => {
                (name.clone(), p.on_surface)
            }
            Some(Place::Nowhere) | None => ("No projector".to_string(), p.on_surface_variant),
        };
        body = body.push(heading("SOUND AND SCREEN")).push(card(
            column![
                sound,
                row![
                    look::icon(Icon::Screen, 16.0, screen.1),
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
}
