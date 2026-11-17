//! Filling a section from the library: the panel that asks, the rows
//! that propose, and the way to take what is proposed into the plan.
//!
//! It proposes and the operator accepts. What it picks shows in the
//! section as rows that are not steps yet; one can be kept through a new
//! draw, or left out, and nothing is in the plan until it is accepted.
//! The choosing itself is [`crate::fill`].

use std::sync::Arc;
use std::time::Duration;

use iced::widget::{Space, button, column, container, row, text, tooltip};
use iced::{Alignment, Element, Fill, Length};
use lantern_engine::{Action, Kind, Resource, ResourceId, StepId};

use crate::fill::{Chosen, Fit, Order, Thing, Wish};
use crate::look::{self, BODY, BOLD, Icon, META, Tone};
use crate::widgets::{self, Filled, hinted};
use crate::{App, Message, drawn, length};

/// The kinds of file a fill can take, in the order their switches are.
const KINDS: [Kind; 3] = [Kind::Video, Kind::Audio, Kind::Image];

/// Something used within this many weeks was on lately: the usual order
/// leaves it out.
const LATELY: i64 = 2;

/// A fill under way: what is asked, and what is proposed for it.
pub struct Filling {
    /// The section being filled.
    pub section: StepId,
    /// The library folder it is filled from, by its place.
    folder: usize,
    /// Which kinds of file may be picked.
    kinds: [bool; 3],
    order: Order,
    fit: Fit,
    margin: Duration,
    /// The time to fill, and whether it is asked for.
    time: Duration,
    time_on: bool,
    /// The number of things, and whether it is asked for.
    count: usize,
    count_on: bool,
    /// Picks that stay through a new draw.
    kept: Vec<ResourceId>,
    /// Things left out by hand.
    out: Vec<ResourceId>,
    seed: u64,
    /// What is proposed now.
    picks: Vec<Arc<Resource>>,
    chosen: Chosen,
    /// How many files could not be offered for not being measured yet.
    measuring: usize,
}

/// One change to a fill.
#[derive(Debug, Clone)]
pub enum FillEdit {
    Open(StepId),
    Close,
    Folder(usize),
    Kind(usize),
    Order(Order),
    Fit(Fit),
    /// The margin, a step up or down.
    Margin(bool),
    TimeOn,
    /// The time, a step up or down.
    Time(bool),
    CountOn,
    /// The number, one more or fewer.
    Count(bool),
    /// Draw again.
    Again,
    /// More of the library has been measured: choose again, same draw.
    Measured,
    Keep(ResourceId),
    Out(ResourceId),
    /// Take what is proposed into the plan.
    Accept,
}

const STEP: Duration = Duration::from_secs(30);

impl App {
    /// Whether a fill is waiting on files that are still being measured.
    pub(crate) fn fill_measuring(&self) -> bool {
        self.filling.as_ref().is_some_and(|fill| fill.measuring > 0)
    }

    /// How long the steps of `section` add up to, and its time.
    fn section_room(&self, section: StepId) -> (Duration, Option<Duration>) {
        let plan = self.engine.0.plan();
        let budget = match plan.get(section).map(|step| &step.action) {
            Some(Action::Section { budget, .. }) => *budget,
            _ => None,
        };
        (self.planned(&plan, section), budget)
    }

    pub(crate) fn fill_edit(&mut self, edit: FillEdit) {
        match edit {
            FillEdit::Open(section) => {
                // A fill has the side to itself.
                self.plan_panel = false;
                self.plan_typed = None;
                let (planned, budget) = self.section_room(section);
                let any_used = self
                    .shelf
                    .iter()
                    .flat_map(|folder| &folder.files)
                    .any(|file| self.usage.times(&file.id) > 0);
                self.filling = Some(Filling {
                    section,
                    folder: 0,
                    kinds: [true, true, false],
                    order: if any_used { Order::Usual } else { Order::Drawn },
                    fit: Fit::AtMost,
                    margin: Duration::from_secs(60),
                    // What is left of the section's time; without one,
                    // a number of things is what there is to ask.
                    time: budget.map_or(Duration::from_secs(10 * 60), |budget| {
                        budget.saturating_sub(planned)
                    }),
                    time_on: budget.is_some(),
                    count: 4,
                    count_on: budget.is_none(),
                    kept: Vec::new(),
                    out: Vec::new(),
                    seed: std::time::SystemTime::now()
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .map_or(1, |since| since.as_nanos() as u64),
                    picks: Vec::new(),
                    chosen: Chosen {
                        picks: Vec::new(),
                        total: Duration::ZERO,
                        each_still: None,
                        off: Duration::ZERO,
                    },
                    measuring: 0,
                });
            }
            FillEdit::Close => {
                self.filling = None;
                return;
            }
            FillEdit::Accept => {
                self.fill_accept();
                return;
            }
            edit => {
                let Some(fill) = &mut self.filling else {
                    return;
                };
                match edit {
                    FillEdit::Folder(folder) => {
                        fill.folder = folder;
                        fill.kept.clear();
                    }
                    FillEdit::Kind(kind) => {
                        if let Some(on) = fill.kinds.get_mut(kind) {
                            *on = !*on;
                        }
                    }
                    FillEdit::Order(order) => fill.order = order,
                    FillEdit::Fit(fit) => fill.fit = fit,
                    FillEdit::Margin(more) => {
                        fill.margin = match more {
                            true => fill.margin + STEP,
                            false => fill.margin.saturating_sub(STEP),
                        };
                    }
                    FillEdit::TimeOn => fill.time_on = !fill.time_on || !fill.count_on,
                    FillEdit::Time(more) => {
                        fill.time = match more {
                            true => fill.time + STEP,
                            false => fill.time.saturating_sub(STEP),
                        };
                    }
                    FillEdit::CountOn => fill.count_on = !fill.count_on || !fill.time_on,
                    FillEdit::Count(more) => {
                        fill.count = match more {
                            true => fill.count + 1,
                            false => fill.count.saturating_sub(1).max(1),
                        };
                    }
                    FillEdit::Again => {
                        fill.seed = fill
                            .seed
                            .wrapping_mul(6_364_136_223_846_793_005)
                            .wrapping_add(1_442_695_040_888_963_407);
                    }
                    FillEdit::Keep(id) => match fill.kept.iter().position(|kept| *kept == id) {
                        Some(at) => {
                            fill.kept.remove(at);
                        }
                        None => fill.kept.push(id),
                    },
                    FillEdit::Out(id) => {
                        fill.kept.retain(|kept| *kept != id);
                        fill.out.push(id);
                    }
                    FillEdit::Measured | FillEdit::Open(_) | FillEdit::Close | FillEdit::Accept => {
                    }
                }
            }
        }
        self.fill_propose();
    }

    /// Chooses again for what is asked now.
    fn fill_propose(&mut self) {
        let Some(fill) = &self.filling else {
            return;
        };
        let plan = self.engine.0.plan();
        let in_plan: Vec<&ResourceId> = plan
            .steps()
            .iter()
            .filter_map(|step| match &step.action {
                Action::Show { resource } => Some(&resource.id),
                _ => None,
            })
            .collect();
        let today = chrono::Local::now().date_naive();
        let mut measuring = 0;
        // What may be picked: of the kinds asked, not in the plan, not
        // left out, and with a length when it is something that plays.
        let offered: Vec<(Arc<Resource>, Thing)> = self
            .shelf
            .get(fill.folder)
            .map(|folder| folder.files.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|file| {
                KINDS
                    .iter()
                    .zip(fill.kinds)
                    .any(|(kind, on)| on && *kind == file.kind)
            })
            .filter(|file| !in_plan.contains(&&file.id) && !fill.out.contains(&file.id))
            .filter_map(|file| {
                let length = match file.kind {
                    Kind::Image => None,
                    _ => match self.file_length(file) {
                        Some(length) => Some(length),
                        None => {
                            measuring += 1;
                            return None;
                        }
                    },
                };
                let kept = fill.kept.contains(&file.id);
                let times = self.usage.times(&file.id);
                // On lately: the usual order leaves it for another week.
                let lately = self
                    .usage
                    .weeks_since(&file.id, today)
                    .is_some_and(|weeks| weeks < LATELY);
                if fill.order == Order::Usual && lately && !kept {
                    return None;
                }
                Some((
                    file.clone(),
                    Thing {
                        length,
                        weight: times as f32,
                    },
                ))
            })
            .collect();
        let things: Vec<Thing> = offered.iter().map(|(_, thing)| *thing).collect();
        let kept: Vec<usize> = fill
            .kept
            .iter()
            .filter_map(|id| offered.iter().position(|(file, _)| &file.id == id))
            .collect();
        let wish = Wish {
            time: fill.time_on.then_some(fill.time),
            count: fill.count_on.then_some(fill.count),
            fit: fill.fit,
            margin: fill.margin,
        };
        let chosen = crate::fill::choose(&things, wish, fill.order, &kept, fill.seed);
        let picks = chosen
            .picks
            .iter()
            .map(|&at| offered[at].0.clone())
            .collect();
        if let Some(fill) = &mut self.filling {
            fill.picks = picks;
            fill.chosen = chosen;
            fill.measuring = measuring;
        }
    }

    /// What is proposed goes into the plan, at the end of its section.
    fn fill_accept(&mut self) {
        let Some(fill) = self.filling.take() else {
            return;
        };
        if fill.picks.is_empty() {
            return;
        }
        self.remember(None);
        let each = fill.chosen.each_still;
        let added = self.engine.0.edit_plan(|plan| {
            let at = plan.position(fill.section)?;
            let mut last = None;
            for (end, pick) in (plan.group_end(at)..).zip(&fill.picks) {
                let id = plan.insert(
                    end,
                    pick.title.clone(),
                    Action::Show {
                        resource: (**pick).clone(),
                    },
                );
                if let Some(step) = plan.get_mut(id) {
                    // Put here by the fill, not picked: showing it says
                    // nothing of what is usually used.
                    step.filled = true;
                    // Something still gets its share of the time.
                    if pick.kind == Kind::Image {
                        step.estimate = each;
                    }
                }
                last = Some(id);
            }
            last
        });
        if let Some(step) = added {
            self.motion.changed(step);
        }
    }

    /// The fill's panel, in the side panel's place: what to fill to, from
    /// where, of what kinds, in what order, and how near the time.
    pub(crate) fn fill_card(&self) -> Option<Element<'_, Message>> {
        let fill = self.filling.as_ref()?;
        let p = self.palette;
        let edit = |edit: FillEdit| Message::Fill(edit);
        let ground = move |_: &iced::Theme| container::Style {
            background: Some(p.surface_container_highest.into()),
            border: iced::Border {
                radius: 14.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        };
        let ink = |on: bool| widgets::place_ink(p, on);
        // One place of a switch.
        let place =
            |content: Element<'static, Message>, on: bool, about: &str, message: Message| {
                widgets::place(p, content, on, 5, about, message)
            };
        let word = |label: &'static str, on: bool| -> Element<'static, Message> {
            text(label).size(BODY).font(BOLD).color(ink(on)).into()
        };
        let glyph = |icon: Icon, on: bool| -> Element<'static, Message> {
            look::icon(icon, 16.0, ink(on)).into()
        };
        let round = |label: &'static str, message: Message| {
            button(
                container(text(label).size(BODY).color(p.on_surface))
                    .center_x(26)
                    .center_y(26),
            )
            .padding(0)
            .on_press(message)
            .style(look::pill(p, Tone::Plain))
        };
        // The two things to fill to, each with its switch and its steps.
        let target = |icon: Icon,
                      on: bool,
                      shown: String,
                      turn: FillEdit,
                      fewer: FillEdit,
                      more: FillEdit,
                      about: &str| {
            let mark = if on { p.primary } else { p.on_surface_variant };
            container(
                column![
                    row![
                        look::icon(icon, 14.0, mark),
                        Space::new().width(Fill),
                        hinted(
                            p,
                            button(text(if on { "on" } else { "off" }).size(META).font(BOLD))
                                .padding([2, 8])
                                .on_press(edit(turn))
                                .style(look::pill(p, if on { Tone::Solid } else { Tone::Plain })),
                            about.to_string(),
                            tooltip::Position::Bottom,
                        ),
                    ]
                    .align_y(Alignment::Center),
                    row![
                        round("−", edit(fewer)),
                        container(text(shown).size(20).font(BOLD).color(if on {
                            p.on_surface
                        } else {
                            p.on_surface_variant
                        }))
                        .center_x(Fill),
                        round("+", edit(more)),
                    ]
                    .align_y(Alignment::Center),
                ]
                .spacing(8),
            )
            .width(Fill)
            .padding(10)
            .style(ground)
        };
        let targets = row![
            target(
                Icon::Clock,
                fill.time_on,
                length::clock(fill.time),
                FillEdit::TimeOn,
                FillEdit::Time(false),
                FillEdit::Time(true),
                "Fill to a time",
            ),
            target(
                Icon::Count,
                fill.count_on,
                fill.count.to_string(),
                FillEdit::CountOn,
                FillEdit::Count(false),
                FillEdit::Count(true),
                "Fill to a number of things",
            ),
        ]
        .spacing(8);
        let switch = |places: Vec<Element<'static, Message>>| widgets::places(p, places, 14.0);
        let kinds = switch(vec![
            place(
                glyph(Icon::Video, fill.kinds[0]),
                fill.kinds[0],
                "Videos",
                edit(FillEdit::Kind(0)),
            ),
            place(
                glyph(Icon::Music, fill.kinds[1]),
                fill.kinds[1],
                "Sound files",
                edit(FillEdit::Kind(1)),
            ),
            place(
                glyph(Icon::Image, fill.kinds[2]),
                fill.kinds[2],
                "Pictures",
                edit(FillEdit::Kind(2)),
            ),
        ]);
        // Where from: each folder of the library.
        let mut folders = column![].spacing(2);
        for (at, folder) in self.shelf.iter().enumerate() {
            let on = at == fill.folder;
            let colour = if on { p.primary } else { p.on_surface };
            folders = folders.push(
                button(
                    row![
                        look::icon(Icon::Folder, 14.0, colour),
                        container(
                            text(folder.label.clone())
                                .size(BODY)
                                .font(if on { BOLD } else { look::MEDIUM })
                                .color(colour)
                                .wrapping(text::Wrapping::None)
                        )
                        .width(Fill)
                        .clip(true),
                        text(folder.files.len().to_string())
                            .size(META)
                            .color(colour),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .width(Fill)
                .padding([6, 10])
                .on_press(edit(FillEdit::Folder(at)))
                .style(look::row(p, if on { Tone::Selected } else { Tone::Plain })),
            );
        }
        let order = switch(vec![
            place(
                word("↓", fill.order == Order::Held),
                fill.order == Order::Held,
                "In the order the library holds them",
                edit(FillEdit::Order(Order::Held)),
            ),
            place(
                glyph(Icon::Shuffle, fill.order == Order::Drawn),
                fill.order == Order::Drawn,
                "Drawn at random",
                edit(FillEdit::Order(Order::Drawn)),
            ),
            place(
                meter(
                    4,
                    ink(fill.order == Order::Usual),
                    ink(fill.order == Order::Usual),
                ),
                fill.order == Order::Usual,
                "What you usually use, leaving out what was on lately",
                edit(FillEdit::Order(Order::Usual)),
            ),
        ]);
        let fit = |fit: Fit, label: &'static str, about: &str| {
            place(
                word(label, fill.fit == fit),
                fill.fit == fit,
                about,
                edit(FillEdit::Fit(fit)),
            )
        };
        let fits = switch(vec![
            fit(Fit::AtMost, "≤", "Never over the time"),
            fit(Fit::Exact, "=", "As near the time as can be"),
            fit(Fit::AtLeast, "≥", "Never under the time"),
            fit(Fit::Either, "±", "Either side of the time"),
        ]);
        let window = Wish {
            time: fill.time_on.then_some(fill.time),
            count: None,
            fit: fill.fit,
            margin: fill.margin,
        }
        .window();
        let mut margin = row![
            round("−", edit(FillEdit::Margin(false))),
            text(length::clock(fill.margin)).size(BODY).font(BOLD),
            round("+", edit(FillEdit::Margin(true))),
            Space::new().width(Fill),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if let Some((least, most)) = window {
            margin = margin.push(
                text(format!(
                    "{} – {}",
                    length::clock(least),
                    length::clock(most)
                ))
                .size(META)
                .color(p.on_surface_variant),
            );
        }
        let mut card = column![
            row![
                look::icon(Icon::Fill, 16.0, p.primary),
                text("Fill").size(BODY).font(BOLD).color(p.primary),
                Space::new().width(Fill),
                button(text("✕").size(META).color(p.on_surface_variant))
                    .padding([2, 6])
                    .on_press(edit(FillEdit::Close))
                    .style(look::row(p, Tone::Plain)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            targets,
            kinds,
            folders,
            order,
        ]
        .spacing(10);
        if fill.time_on {
            card = card.push(fits).push(margin);
        }
        if fill.measuring > 0 {
            card = card.push(
                row![
                    look::ring(0.3, 14.0, p.tertiary),
                    text(fill.measuring.to_string())
                        .size(META)
                        .color(p.tertiary),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
        Some(iced::widget::scrollable(card).height(Fill).into())
    }

    /// What the fill proposes, as rows under its section that are not
    /// steps yet, and the ways to draw again, give up, or take them.
    pub(crate) fn fill_rows<'a>(&self, indent: f32) -> Option<Element<'a, Message>> {
        let fill = self.filling.as_ref()?;
        let p = self.palette;
        let edit = |edit: FillEdit| Message::Fill(edit);
        let today = chrono::Local::now().date_naive();
        let (planned, budget) = self.section_room(fill.section);
        // Over the rows: what is there, what is proposed, how far off.
        let mut sums = row![
            text(length::clock(planned))
                .size(META)
                .color(p.on_surface_variant),
            text(format!("+ {}", length::clock(fill.chosen.total)))
                .size(META)
                .font(BOLD)
                .color(p.primary),
            Space::new().width(Fill),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if fill.count_on {
            sums = sums.push(
                text(format!("# {} / {}", fill.picks.len(), fill.count))
                    .size(META)
                    .color(if fill.picks.len() == fill.count {
                        p.primary
                    } else {
                        p.tertiary
                    }),
            );
        }
        if !fill.chosen.off.is_zero() {
            sums = sums.push(
                text(format!("± {}", length::clock(fill.chosen.off)))
                    .size(META)
                    .color(p.tertiary),
            );
        }
        // Over them, as a bar: what is there, what is proposed (hatched:
        // not added yet), and what room is left of the section's time.
        let whole = budget
            .unwrap_or_default()
            .max(planned + fill.chosen.total)
            .as_secs_f32()
            .max(1.0);
        let share = |of: Duration| of.as_secs_f32() / whole;
        let (there, proposed) = (share(planned), share(fill.chosen.total));
        // Only the parts there are: a part of nothing would still be a
        // sliver.
        let parts = widgets::bar(
            [
                (there, Filled::Solid(p.primary)),
                (
                    proposed,
                    Filled::Hatched(
                        iced::Color {
                            a: 0.75,
                            ..p.primary
                        },
                        iced::Color {
                            a: 0.4,
                            ..p.primary
                        },
                    ),
                ),
                (
                    1.0 - there - proposed,
                    Filled::Solid(p.surface_container_highest),
                ),
            ]
            .into_iter()
            .filter(|(share, _)| *share >= 0.0005),
            8.0,
        );
        let mut rows = column![parts, sums].spacing(6);
        for pick in &fill.picks {
            let kept = fill.kept.contains(&pick.id);
            let times = self.usage.times(&pick.id);
            let weeks = self.usage.weeks_since(&pick.id, today);
            let (level, colour, since) = match times {
                0 => (0, p.tertiary, "new".to_string()),
                times => (
                    match times {
                        1 => 1,
                        2..=3 => 2,
                        4..=7 => 3,
                        _ => 4,
                    },
                    p.primary,
                    weeks.map_or_else(String::new, |weeks| format!("{weeks}w")),
                ),
            };
            let long = match pick.kind {
                Kind::Image => fill.chosen.each_still,
                _ => self.file_length(pick),
            }
            .map(length::clock)
            .unwrap_or_default();
            rows = rows.push(drawn::dashed(
                container(
                    row![
                        drawn::dashed_dot(10.0, p.primary),
                        container(
                            text(pick.title.clone())
                                .size(BODY)
                                .color(p.primary)
                                .wrapping(text::Wrapping::None)
                        )
                        .width(Fill)
                        .clip(true),
                        meter(level, colour, p.outline_variant),
                        text(since).size(META).color(colour),
                        text(long).size(META).color(p.on_surface_variant),
                        hinted(
                            p,
                            button(
                                text(if kept { "●" } else { "○" })
                                    .size(META)
                                    .color(p.primary)
                            )
                            .padding([2, 6])
                            .on_press(edit(FillEdit::Keep(pick.id.clone())))
                            .style(look::row(
                                p,
                                if kept { Tone::Selected } else { Tone::Plain }
                            )),
                            if kept {
                                "Stays when you draw again. Click to let it go".to_string()
                            } else {
                                "Keep it when you draw again".to_string()
                            },
                            tooltip::Position::Top,
                        ),
                        button(text("✕").size(META).color(p.on_surface_variant))
                            .padding([2, 6])
                            .on_press(edit(FillEdit::Out(pick.id.clone())))
                            .style(look::row(p, Tone::Plain)),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([6, 8]),
                iced::Color {
                    a: 0.45,
                    ..p.primary
                },
                12.0,
            ));
        }
        let some = !fill.picks.is_empty();
        rows = rows.push(
            row![
                hinted(
                    p,
                    button(look::icon(Icon::Shuffle, 16.0, p.on_surface))
                        .padding([6, 12])
                        .on_press(edit(FillEdit::Again))
                        .style(look::pill(p, Tone::Plain)),
                    "Draw again, keeping what is kept".to_string(),
                    tooltip::Position::Top,
                ),
                Space::new().width(Fill),
                button(text("✕").size(BODY).color(p.on_surface_variant))
                    .padding([6, 12])
                    .on_press(edit(FillEdit::Close))
                    .style(look::row(p, Tone::Plain)),
                button(
                    row![
                        look::icon(Icon::Check, 16.0, p.on_primary),
                        text(fill.picks.len().to_string())
                            .size(BODY)
                            .font(BOLD)
                            .color(p.on_primary)
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .padding([6, 14])
                .on_press_maybe(some.then(|| edit(FillEdit::Accept)))
                .style(look::pill(p, Tone::Solid)),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
        Some(
            container(rows)
                .padding(iced::Padding::default().left(indent).top(4.0).bottom(6.0))
                .into(),
        )
    }
}

/// How often something is used, as four bars: `level` of them lit.
fn meter(level: usize, lit: iced::Color, unlit: iced::Color) -> Element<'static, Message> {
    let bar = |at: usize, tall: f32| {
        let colour = if at < level { lit } else { unlit };
        container(Space::new().width(3).height(tall))
            .style(move |_| container::Style::default().background(colour))
    };
    row![bar(0, 4.0), bar(1, 7.0), bar(2, 9.5), bar(3, 12.0)]
        .spacing(1.5)
        .align_y(Alignment::End)
        .height(Length::Fixed(12.0))
        .into()
}
