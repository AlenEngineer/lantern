//! The operator's window: three cards on one ground. A strip across the
//! top (the plan, the service's time, the tools), the plan on the left, and
//! the stage in the middle: what the audience sees, and beside it what's
//! next or lined up. The search drops from the strip as a panel.

use std::time::{Duration, SystemTime};

use iced::widget::{
    Space, button, column, container, image, pin, row, scrollable, slider, stack, text, text_input,
};
use iced::{Alignment, Color, Element, Fill, Length};
use lantern_engine::{Action, Bang, Media, Playback, Resource, Scene, Scope, Step, Timed, Timing};

use crate::fit;
use crate::keys::{self, Op};
use crate::length;
use crate::look::{self, BODY, BOLD, HEADING, Icon, MEDIUM, META, PILL, Tone};
use crate::palette::Palette;
use crate::plan_pane::section_status;
use crate::ready::{self, Issue, Level};
use crate::search::{Group, Live, supported};
use crate::stage;
use crate::stage::Content;
use crate::youtube::YoutubeVideo;
use crate::{App, Clicked, Message};

pub const SEARCH_BOX: &str = "search";
pub const RESULTS: &str = "results";
pub const NAME_BOX: &str = "plan-name";
/// The field for the path of a plan to import.
pub const IMPORT_BOX: &str = "plan-import";

/// The plan pane's and the next column's widths with room to spare, and
/// the least they shrink to in a narrow window.
const PLAN_WIDTH: (f32, f32) = (340.0, 280.0);
const STAGE_WIDTH: (f32, f32) = (280.0, 220.0);
/// What the live picture keeps before the side columns start to shrink.
const LAUNCHER_ROOM: f32 = 560.0;
/// A side column dragged to a width stays between these, and leaves the
/// live picture at least its own.
const PANE_LEAST: f32 = 200.0;
const PANE_MOST: f32 = 560.0;
const MIDDLE_LEAST: f32 = 320.0;
/// How wide the line between panes is to the pointer.
const SEAM_GRIP: f32 = 10.0;
/// The window's padding, the gap between its cards, a card's padding, and
/// the strip's height.
const EDGE: f32 = 12.0;
const GAP: f32 = 10.0;
const CARD_PADDING: f32 = 16.0;
const STRIP: f32 = 48.0;
/// How wide the search panel is, where the window has the room.
const SEARCH_PANEL: f32 = 460.0;

/// A side pane, by the edge that sizes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Plan,
    Stage,
}

/// The most the settings spread.
const SETTINGS_WIDTH: f32 = 880.0;

/// The plan and stage panes' widths in a window `width` wide: their full
/// widths while the launcher has room, then both narrower by the same
/// share, down to their least.
pub(crate) fn pane_widths(width: f32) -> (f32, f32) {
    let (plan, stage) = (PLAN_WIDTH.0, STAGE_WIDTH.0);
    let share = ((width - LAUNCHER_ROOM) / (plan + stage)).min(1.0);
    (
        (plan * share).max(PLAN_WIDTH.1).round(),
        (stage * share).max(STAGE_WIDTH.1).round(),
    )
}
/// What the stage pane's estimate of its own height may be short by.
const FIT_SLACK: f32 = 24.0;
/// How wide a result's thumbnail is.
const RESULT_THUMB: f32 = 96.0;

impl App {
    /// The plan pane's and the next column's widths: as dragged, else as
    /// they come for the window, and never so wide that the live picture
    /// loses its room.
    fn side_widths(&self) -> (f32, f32) {
        let window = self.dashboard_size.width;
        let (plan, stage) = pane_widths(window);
        let most = |other: f32| (window - other - MIDDLE_LEAST).max(PANE_LEAST);
        let plan = self
            .plan_dragged
            .map_or(plan, |w| w.clamp(PANE_LEAST, PANE_MOST));
        let stage = self
            .stage_dragged
            .map_or(stage, |w| w.clamp(PANE_LEAST, PANE_MOST));
        (plan.min(most(stage)), stage.min(most(plan)))
    }

    pub(crate) fn plan_width(&self) -> f32 {
        self.side_widths().0
    }

    /// The column beside the live picture: what's next, or the search
    /// while it's open.
    fn side_width(&self) -> f32 {
        if self.search_open {
            self.search_width()
        } else {
            self.side_widths().1
        }
    }

    /// The search panel's width: its own, less in a narrow window so the
    /// live picture keeps some room.
    fn search_width(&self) -> f32 {
        let room = self.dashboard_size.width - self.plan_width() - MIDDLE_LEAST;
        SEARCH_PANEL.min(room).max(PANE_LEAST)
    }

    /// Where the pointer has dragged the edge of a side pane to.
    pub(crate) fn dragged_to(&self, side: Side, x: f32) -> f32 {
        let width = match side {
            Side::Plan => x - EDGE,
            Side::Stage => self.dashboard_size.width - x - EDGE - CARD_PADDING,
        };
        width.clamp(PANE_LEAST, PANE_MOST)
    }

    /// The live picture's room: what the plan, the side column and the
    /// gaps leave of the window's width.
    fn live_width(&self) -> f32 {
        let taken = 2.0 * EDGE
            + self.plan_width()
            + GAP
            + 2.0 * CARD_PADDING
            + SEAM_GRIP
            + self.side_width();
        (self.dashboard_size.width - taken).max(120.0)
    }

    /// How see-through the dashboard is now, from 0 (solid) to 1: as the
    /// settings' slider has it while they're open, else as saved.
    pub(crate) fn see_through(&self) -> f32 {
        let percent = match &self.settings {
            Some(draft) => draft.see_through,
            None => self.config.see_through,
        };
        f32::from(percent.min(crate::config::MOST_SEE_THROUGH)) / 100.0
    }

    pub(crate) fn dashboard_view(&self) -> Element<'_, Message> {
        let p = self.palette;
        let glass = self.see_through();
        // The gap between two cards is a handle: drag it to size the side
        // it bounds, double-click it to let that size itself again. A line
        // shows in it while it's held.
        let seam = |side: Side, width: f32| {
            let held = self.resizing == Some(side);
            let line = container(Space::new())
                .width(2)
                .height(Fill)
                .style(move |_| match held {
                    true => container::Style::default().background(p.primary),
                    false => container::Style::default(),
                });
            iced::widget::mouse_area(container(line).center_x(width).height(Fill))
                .on_press(Message::ResizePane(side))
                .interaction(iced::mouse::Interaction::ResizingHorizontally)
        };
        let timing = self.timing();
        let plan = container(self.plan_pane(&timing))
            .height(Fill)
            .style(look::card(p, glass, look::CARD_RADIUS));
        // The settings take the stage's card while they're open.
        let middle: Element<'_, Message> = match &self.settings {
            Some(draft) => container(self.settings_view(draft))
                .max_width(SETTINGS_WIDTH)
                .into(),
            None => self.stage_centre(&seam),
        };
        let stage = container(middle)
            .center_x(Fill)
            .height(Fill)
            .padding(CARD_PADDING)
            .style(look::card(p, glass, look::CARD_RADIUS));
        let window = container(
            column![
                self.strip(&timing),
                row![plan, seam(Side::Plan, GAP), stage].height(Fill)
            ]
            .spacing(GAP),
        )
        .width(Fill)
        .height(Fill)
        .padding(EDGE)
        .style(look::window(p, glass));
        let mut layers = stack![window];
        // The search drops from the strip, over the room the stage's card
        // leaves it beside the live picture.
        if self.search_open && self.settings.is_none() {
            let size = self.dashboard_size;
            let panel = container(self.search_panel())
                .width(self.search_width() + CARD_PADDING)
                .height((size.height - EDGE - STRIP - EDGE).max(160.0))
                .padding(CARD_PADDING)
                .style(look::panel(p, glass));
            layers = layers.push(
                pin(panel)
                    .x(size.width - EDGE - self.search_width() - CARD_PADDING)
                    .y(EDGE + STRIP),
            );
        }
        // A moving drag's card follows the pointer, just off to its side so
        // the rows under the pointer still know it's there.
        if let Some(drag) = self.drag.as_ref().filter(|drag| drag.moving) {
            let card = container(
                text(drag.label(&self.engine.0.plan()))
                    .size(BODY)
                    .font(BOLD)
                    .color(p.primary),
            )
            .padding([10, 14])
            .max_width(320)
            .style(move |_| container::Style {
                background: Some(p.surface_container_highest.into()),
                border: iced::Border {
                    color: p.primary,
                    width: 1.0,
                    radius: 14.0.into(),
                },
                shadow: iced::Shadow {
                    color: Color::BLACK.scale_alpha(0.4),
                    offset: iced::Vector::new(0.0, 4.0),
                    blur_radius: 12.0,
                },
                ..container::Style::default()
            });
            layers = layers.push(pin(card).x(drag.at.x + 16.0).y(drag.at.y + 8.0));
        }
        layers.into()
    }

    // The launcher

    /// The plan's timing now, with the lengths of the videos loaded so far.
    fn timing(&self) -> Timing {
        self.engine.0.timing(&|step| self.loaded_length(step))
    }

    /// What in `plan` isn't ready to show (see [`ready`](crate::ready)).
    pub(crate) fn issues(&self, plan: &lantern_engine::Plan) -> Vec<Issue> {
        ready::check(
            plan,
            |video| self.downloads.0.state(&video.id),
            stage::image_cached,
        )
    }

    /// The plan's readiness as a chip ("ready", "1 problem"), coloured by
    /// how bad it is; `None` for a plan that shows nothing.
    pub(crate) fn readiness<'a>(
        &self,
        plan: &lantern_engine::Plan,
        issues: &[Issue],
    ) -> Option<Element<'a, Message>> {
        let (text, level) = ready::summary(plan, issues)?;
        Some(chip(self.palette, &text, level_tone(level)))
    }

    /// A step's media length, once the stage has loaded it.
    pub(crate) fn loaded_length(&self, step: &Step) -> Option<Duration> {
        match &step.action {
            Action::Show { resource } => self.media.length(&resource.id),
            _ => None,
        }
    }

    /// The strip across the top: every item says how something stands
    /// and opens what changes it. The plan and its place on the left, the
    /// service's time in the middle, the tools on the right.
    fn strip(&self, timing: &Timing) -> Element<'_, Message> {
        let p = self.palette;
        let wide = self.dashboard_size.width;
        let plan = self.engine.0.plan();
        let tool = |icon: Icon, label: &str, tone: Tone, message: Message| {
            let colour = match tone {
                Tone::Plain => p.on_surface_variant,
                tone => tone.text(p),
            };
            let mut inside = row![look::icon(icon, 15.0, colour)]
                .spacing(8)
                .align_y(Alignment::Center);
            if !label.is_empty() {
                inside = inside.push(
                    text(label.to_string())
                        .size(BODY)
                        .font(MEDIUM)
                        .color(colour)
                        .wrapping(text::Wrapping::None),
                );
            }
            button(inside)
                .padding([9, 12])
                .on_press(message)
                .style(look::strip_item(p, tone))
        };
        let mut strip = row![container(self.plan_header(&plan)).width(self.plan_width() - 8.0)]
            .spacing(4)
            .align_y(Alignment::Center);
        strip = strip.push(Space::new().width(Fill));
        // The running section and how the service stands against its
        // schedule: behind or ahead with a start time, else when it ends.
        let ends_in = |end: SystemTime| end.duration_since(SystemTime::now()).unwrap_or_default();
        let (starts, scheduled) = self.engine.0.edit_plan(|plan| {
            let total = plan.starts.map(|_| {
                lantern_engine::timing::schedule(plan, &|step| self.loaded_length(step)).total
            });
            (plan.starts, total.filter(|_| !plan.is_empty()))
        });
        let drift = starts
            .zip(scheduled)
            .zip(timing.end)
            .map(|((starts, total), end)| {
                let behind = lantern_engine::timing::behind(
                    starts,
                    total,
                    length::now_of_day(),
                    ends_in(end),
                );
                length::drift(behind)
            });
        let said = |words: String| {
            text(words)
                .size(BODY)
                .color(p.on_surface)
                .wrapping(text::Wrapping::None)
        };
        match (timing.running(), timing.end) {
            (Some(section), end) => {
                let (status, tone) = section_status(section);
                strip = strip.push(chip(p, &status, tone));
                if let Some(drift) = &drift {
                    strip = strip.push(container(said(drift.clone())).padding([0, 10]));
                } else if let Some(end) = end {
                    strip = strip.push(
                        container(said(format!(
                            "ends in {}",
                            length::rough(ends_in(end)).trim_start_matches('~')
                        )))
                        .padding([0, 10]),
                    );
                }
            }
            // Under way outside any section.
            (None, Some(_)) => {
                if let Some(drift) = &drift {
                    strip = strip.push(container(said(drift.clone())).padding([0, 10]));
                }
            }
            // Not begun: how long until it should.
            (None, None) => {
                let waiting = starts.zip(scheduled).and_then(|(starts, _)| {
                    let now = length::now_of_day().as_secs() as i64;
                    let until = starts.since_midnight().as_secs() as i64 - now;
                    length::starting(&starts.to_string(), until)
                });
                if let Some(waiting) = waiting {
                    strip = strip.push(container(said(waiting)).padding([0, 10]));
                }
            }
        }
        // The time of day, where there's room for it.
        if wide >= 1100.0 {
            strip = strip.push(
                container(
                    text(length::clock_of_day(length::now_of_day()))
                        .size(BODY)
                        .font(BOLD)
                        .color(p.on_surface_variant),
                )
                .padding([0, 10]),
            );
        }
        strip = strip.push(Space::new().width(Fill));
        let search_tone = if self.search_open {
            Tone::Selected
        } else {
            Tone::Plain
        };
        strip = strip.push(tool(
            Icon::Search,
            if wide >= 1000.0 { "Search" } else { "" },
            search_tone,
            op(Op::Search),
        ));
        // What isn't ready, while something isn't.
        let issues = self.issues(&plan);
        if !issues.is_empty()
            && let Some(readiness) = self.readiness(&plan, &issues)
        {
            strip = strip.push(readiness);
        }
        // The screen the stage is on, where one was chosen.
        if let Some(output) = self
            .config
            .stage_output
            .as_deref()
            .filter(|_| wide >= 1180.0)
        {
            strip = strip.push(tool(
                Icon::Video,
                output,
                Tone::Plain,
                Message::Settings(crate::settings::Edit::Category(
                    crate::settings::Category::Output,
                )),
            ));
        }
        // Blackout: solid while the projector is black.
        let blackout = self.stage.blackout;
        strip = strip.push(
            button(
                text(if blackout {
                    "● Blackout ON"
                } else {
                    "Blackout"
                })
                .size(BODY)
                .font(MEDIUM)
                .wrapping(text::Wrapping::None),
            )
            .padding([9, 14])
            .on_press(Message::Key {
                op: Op::Blackout,
                only_if_empty: false,
            })
            .style(look::pill(
                p,
                if blackout { Tone::Alarm } else { Tone::Danger },
            )),
        );
        strip = strip.push(tool(Icon::Settings, "", Tone::Plain, op(Op::Settings)));
        container(strip)
            .height(STRIP)
            .padding([0, 8])
            .align_y(Alignment::Center)
            .style(look::card(p, self.see_through(), look::PILL_RADIUS))
            .into()
    }

    /// The search, as the panel that drops from the strip: the box, what
    /// it found, and under them the result being looked at with what can
    /// be done with it.
    fn search_panel(&self) -> Element<'_, Message> {
        let mut panel = column![self.search_box(), self.results()].spacing(10);
        if self.stage.preview.is_some() {
            let mut preview = self.preview_section();
            // The picture small: the list above is what's being read. In
            // a short window it's left out, and the list keeps its room.
            let monitor = std::mem::replace(&mut preview[1], Space::new().into());
            let roomy = self.dashboard_size.height >= 640.0;
            preview[1] = match roomy {
                true => container(container(monitor).width(240).height(135))
                    .center_x(Fill)
                    .into(),
                false => Space::new().into(),
            };
            panel = panel.push(column(preview.into_iter().skip(1)).spacing(8));
        }
        panel.height(Fill).into()
    }

    /// The config, the index and the colours, only when something is wrong
    /// or still loading.
    fn problems(&self) -> Vec<String> {
        let mut problems: Vec<String> = self.config_status.problem().into_iter().collect();
        problems.extend(self.library.0.problems());
        problems.extend(self.palette_error.clone());
        problems
    }

    pub(crate) fn search_box(&self) -> Element<'_, Message> {
        let p = self.palette;
        // A bang that became the chip; an unknown one stays as text, and
        // the results say no source answers it.
        let (scope, placeholder): (Element<'_, Message>, String) =
            match (&self.search.scope, self.search.bang()) {
                (Scope::Bang(bang), Some(_)) => {
                    let sources = bang.sources.join(", ");
                    let placeholder = format!("Search {sources} · Backspace for everything");
                    (chip(p, &sources, Tone::Plain), placeholder)
                }
                // The whole hint only where it fits.
                _ if self.search_width() < 520.0 => (
                    Space::new().into(),
                    "Search, ! for one source, or a link".into(),
                ),
                _ => (
                    Space::new().into(),
                    "Search everything, ! for one source, or paste a link".into(),
                ),
            };
        let input = text_input(&placeholder, &self.search.query)
            .id(SEARCH_BOX)
            // No `on_submit`: Enter comes through the keymap, so Ctrl+Enter
            // can mean something else and Enter can be another key.
            .on_input(Message::Query)
            .size(15)
            .padding([10, 0])
            .style(move |theme, status| text_input::Style {
                background: Color::TRANSPARENT.into(),
                border: iced::Border::default(),
                ..look::input(p)(theme, status)
            });
        // The box keeps the focus, so its ring stays on.
        container(
            row![
                look::icon(Icon::Search, 18.0, p.on_surface_variant),
                scope,
                input
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding([0, 14])
        .style(move |theme| container::Style {
            border: iced::Border {
                color: p.primary,
                width: 2.0,
                radius: look::ROW_RADIUS.into(),
            },
            ..container::Style::default()
                .background(look::input(p)(theme, text_input::Status::Active).background)
        })
        .into()
    }

    pub(crate) fn results(&self) -> Element<'_, Message> {
        let p = self.palette;
        let search = &self.search;
        let bangs = search.bangs(&self.engine.0);
        let offered = bangs.len();
        let mut list = column![].spacing(2);
        if offered > 0 {
            list = list.push(group_heading(p, "Search one source", offered));
            for bang in bangs {
                list = list.push(bang_row(p, bang));
            }
        }
        for group in search.groups() {
            list = list.push(self.group(group));
        }
        if let Some(note) = self.results_note(offered) {
            list = list.push(
                container(text(note).size(BODY).color(p.on_surface_variant)).padding([8, 12]),
            );
        }
        scrolling(list).id(RESULTS).into()
    }

    /// What to say when there are no results to show, or not yet.
    fn results_note(&self, bangs_offered: usize) -> Option<String> {
        let search = &self.search;
        if search.query.trim().is_empty() && search.bang().is_none() {
            return Some("Type to search everything at once.".into());
        }
        if let Some(bang) = search.typing_bang() {
            return (bangs_offered == 0).then(|| format!("No source answers !{bang}."));
        }
        match (&search.scope, search.live) {
            (Scope::Unknown(bang), _) => Some(format!(
                "No source answers !{bang}. Clear the box to see the bangs."
            )),
            (Scope::Bang(bang), Live::OnEnter) => Some(format!(
                "Press {} to search {}.",
                keys::label(Op::Enter),
                bang.sources.join(", ")
            )),
            (_, Live::Waiting(_) | Live::Running(_)) => Some("Still searching…".into()),
            (Scope::Bang(bang), _) if search.text().is_empty() => Some(format!(
                "Type what to look for in {}.",
                bang.sources.join(", ")
            )),
            (_, Live::Idle) if search.groups().is_empty() => {
                Some(format!("Nothing found for “{}”.", search.query.trim()))
            }
            _ => None,
        }
    }

    fn group<'a>(&'a self, group: &'a Group) -> Element<'a, Message> {
        let p = self.palette;
        let mut rows = column![group_heading(p, &group.name, group.hits.len())].spacing(2);
        if let Some(error) = &group.error {
            rows = rows.push(
                container(text(format!("Failed: {error}")).size(BODY).color(p.error))
                    .padding([4, 12]),
            );
        }
        for resource in &group.hits {
            rows = rows.push(self.result_row(resource));
        }
        rows.into()
    }

    fn result_row<'a>(&'a self, resource: &'a Resource) -> Element<'a, Message> {
        let p = self.palette;
        let ok = supported(resource);
        let tone = if self.search.is_picked(resource) {
            Tone::Selected
        } else {
            Tone::Plain
        };
        let mut meta: Vec<String> = resource.subtitle.iter().cloned().collect();
        // A passage says how it starts: that's what tells two verses apart.
        meta.extend(resource.location.as_text().and_then(opening));
        meta.extend(self.download_state(resource));
        if !ok {
            meta.push("can't be shown yet".into());
        }
        let title_color = match (ok, tone) {
            (false, _) => p.outline,
            (true, Tone::Selected) => p.primary,
            (true, _) => p.on_surface,
        };
        let line = row![
            self.thumbnail(resource, RESULT_THUMB),
            column![
                single_line(&resource.title)
                    .size(BODY)
                    .font(if tone == Tone::Selected { BOLD } else { MEDIUM })
                    .color(title_color),
                single_line(meta.join(" · "))
                    .size(META)
                    .color(p.on_surface_variant),
            ]
            .spacing(2),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        button(container(line).clip(true))
            .width(Fill)
            .padding([8, 12])
            .on_press_maybe(ok.then(|| Message::Click(Clicked::Result(resource.id.clone()))))
            .style(look::row(p, tone))
            .into()
    }

    /// A small picture of `resource`, `width` wide at 16:9: a picture, page
    /// or video still once it's made, and the kind's icon until then or
    /// when there's none. A text has the icon: its slide at this size is
    /// unreadable, so every passage looked the same.
    pub(crate) fn thumbnail<'a>(&self, resource: &Resource, width: f32) -> Element<'a, Message> {
        let p = self.palette;
        let height = width * 9.0 / 16.0;
        let made = match resource.location.as_text() {
            Some(_) => None,
            None => self.thumbs.get(resource),
        };
        let picture: Element<'a, Message> = match made {
            Some(handle) => container(
                image(handle)
                    .width(Fill)
                    .height(Fill)
                    .content_fit(iced::ContentFit::Cover),
            )
            .style(|_| container::Style::default().background(Color::BLACK))
            .into(),
            None => container(look::icon(
                Icon::of(&resource.kind),
                height * 0.4,
                p.on_surface_variant,
            ))
            .center(Fill)
            .style(look::chip(p, Tone::Plain))
            .into(),
        };
        container(picture)
            .width(width)
            .height(height)
            .clip(true)
            .style(look::thumb(p))
            .into()
    }

    // The stage pane

    /// The stage's card: what the audience sees, large, and beside it
    /// what comes next (or what's lined up). While the search is open its
    /// panel lies over the side column, which is left empty for it.
    fn stage_centre<'a>(
        &'a self,
        seam: &dyn Fn(Side, f32) -> iced::widget::MouseArea<'a, Message>,
    ) -> Element<'a, Message> {
        let p = self.palette;
        let mut live = self.on_stage_section();
        // The picture is 16:9 of its column where the card is tall enough
        // for the rest too; in a shorter one it takes what the rest leaves.
        let scene = self.stage.program.as_ref();
        let rest = [
            (true, 36.0 + PILL),
            (scene.is_some(), 36.0),
            (
                scene.is_some_and(|s| self.slide_strip(s).is_some()),
                STRIP_HEIGHT,
            ),
            (scene.is_some_and(|s| s.timed().is_some()), PILL),
        ]
        .iter()
        .filter(|(there, _)| *there)
        .map(|(_, height)| height)
        .sum::<f32>()
            + 10.0 * live.len() as f32
            + FIT_SLACK;
        let room = self.dashboard_size.height - 2.0 * EDGE - STRIP - GAP - 2.0 * CARD_PADDING;
        let full = (self.live_width() * 9.0 / 16.0).round();
        let monitor = std::mem::replace(&mut live[1], Space::new().into());
        let slot = container(monitor).width(Fill);
        live[1] = if rest + full <= room {
            slot.height(full).into()
        } else {
            slot.height(Fill).into()
        };
        let mut live = column(live).spacing(10).width(Fill).height(Fill);
        for problem in self.problems() {
            live = live.push(text(problem).size(META).color(p.error));
        }
        // Beside it: the search's room while it's open, else what Enter or
        // PgDn would show (the result or step being looked at, or the
        // plan's next step).
        let side: Element<'_, Message> = if self.search_open {
            Space::new().width(self.search_width()).into()
        } else {
            let mut next = if self.stage.preview.is_some() {
                self.preview_section()
            } else {
                self.next_section()
            };
            let width = self.side_width();
            let monitor = std::mem::replace(&mut next[1], Space::new().into());
            next[1] = container(monitor)
                .width(Fill)
                .height((width * 9.0 / 16.0).round())
                .into();
            column(next).spacing(10).width(width).into()
        };
        row![live, seam(Side::Stage, SEAM_GRIP), side]
            .height(Fill)
            .into()
    }

    fn on_stage_section(&self) -> Vec<Element<'_, Message>> {
        let p = self.palette;
        let scene = self.stage.program.as_ref();
        let blackout = self.stage.blackout;
        let state = match (scene, blackout) {
            (_, true) => chip(p, "blackout", Tone::Danger),
            (Some(_), false) if self.off_plan().is_some() => {
                chip(p, "● live · off the plan", Tone::Live)
            }
            (Some(_), false) => chip(p, "● live", Tone::Live),
            // Without a program the projector still shows the look's image.
            (None, false) if self.look.image.is_some() => chip(p, "background", Tone::Plain),
            (None, false) => chip(p, "empty", Tone::Plain),
        };
        let mut section: Vec<Element<'_, Message>> = vec![
            row![state, Space::new().width(Fill)]
                .align_y(Alignment::Center)
                .into(),
        ];
        section.push(match scene {
            _ if blackout => empty_monitor(
                p,
                &format!(
                    "The projector is black. {} brings it back.",
                    keys::label(Op::Blackout)
                ),
            ),
            Some(scene) => self.monitor(scene, self.media.content(scene.id), Some(p.tertiary)),
            // What the audience sees with nothing on: the look's image.
            None if self.look.image.is_some() => framed_with(
                crate::backdrop(&self.look, Space::new(), crate::Over::Nothing),
                None,
                2,
            ),
            None => empty_monitor(p, "Nothing on the projector"),
        });
        if let Some(scene) = scene {
            section.push(self.caption(scene));
            if let Some(strip) = self.slide_strip(scene) {
                section.push(strip);
            }
            if let Some(timed) = scene.timed() {
                section.push(self.transport(timed));
            }
        }
        // Blackout is in the strip; what's left here empties the stage.
        section.push(
            row![
                Space::new().width(Fill),
                pill(
                    p,
                    "Clear the stage",
                    Tone::Plain,
                    PILL,
                    scene.is_some().then_some(Message::Key {
                        op: Op::Clear,
                        only_if_empty: false,
                    })
                ),
            ]
            .into(),
        );
        section
    }

    fn preview_section(&self) -> Vec<Element<'_, Message>> {
        let p = self.palette;
        let scene = self.stage.preview.as_ref();
        // A selected plan step with nothing to preview (a black screen, a
        // live slot) still lines up here, ready to show.
        let cued = self
            .cued()
            .filter(|step| !matches!(step.action, Action::Show { .. }));
        let mut section: Vec<Element<'_, Message>> = vec![heading(p, "Preview")];
        match (scene, &cued) {
            (Some(scene), _) => {
                let content = self.media.preloaded(&scene.resource.id);
                section.push(self.monitor(scene, content, Some(p.primary)));
                section.push(self.caption(scene));
            }
            (None, Some(step)) => {
                section.push(self.step_picture(step));
                section.push(single_line(step.label.clone()).size(BODY).font(BOLD).into());
            }
            (None, None) => section.push(empty_monitor(p, "Pick a result to preview it here")),
        }
        let has = scene.is_some() || cued.is_some();
        // Looking at a step of the plan (not a search result): adding it
        // would add it a second time, so the button says so.
        let in_plan = self.search.picked().is_none()
            && match (scene, self.cued()) {
                (Some(scene), Some(step)) => matches!(
                    &step.action,
                    Action::Show { resource } if resource.id == scene.resource.id
                ),
                (None, Some(_)) => true,
                _ => false,
            };
        let add = if in_plan {
            keys::with_key("Add again", Op::AddToPlan)
        } else {
            keys::with_key("Add", Op::AddToPlan)
        };
        // One under the other: side by side their keys didn't fit.
        section.push(
            column![
                pill(
                    p,
                    &keys::with_key("Show now", Op::Enter),
                    Tone::Selected,
                    PILL,
                    has.then_some(op(Op::Take))
                )
                .width(Fill),
                pill(p, &add, Tone::Plain, PILL, has.then_some(op(Op::AddToPlan))).width(Fill),
            ]
            .spacing(4)
            .into(),
        );
        section
    }

    fn next_section(&self) -> Vec<Element<'_, Message>> {
        let p = self.palette;
        // Off the plan, what matters is the way back: the step the plan is
        // waiting at, and a button that shows it again.
        if let Some(step) = self.off_plan() {
            let (label, moves) = self.next_label(self.engine.0.up_next().is_some());
            return vec![
                heading(p, "The plan is waiting"),
                self.step_picture(&step),
                single_line(step.label.clone()).size(BODY).font(BOLD).into(),
                row![
                    wide(pill(
                        p,
                        "Back to the plan",
                        Tone::Selected,
                        PILL,
                        Some(Message::Resume)
                    )),
                    pill(p, &label, Tone::Plain, PILL, moves.then_some(op(Op::Next))),
                ]
                .spacing(4)
                .into(),
            ];
        }
        let next = self.engine.0.up_next();
        let mut section: Vec<Element<'_, Message>> = vec![heading(p, "Up next")];
        section.push(match &next {
            Some(step) => self.step_picture(step),
            // Nothing next isn't always the end: there may be no plan, or
            // one with nothing in it to show yet.
            None => {
                let plan = self.engine.0.plan();
                let steps = plan.steps().iter().any(|step| !step.is_section());
                empty_monitor(
                    p,
                    if !self.plans.has_plan(&plan) {
                        "No plan open"
                    } else if !steps {
                        "This plan has no steps yet"
                    } else {
                        "The end of the plan"
                    },
                )
            }
        });
        if let Some(step) = &next {
            section.push(single_line(step.label.clone()).size(BODY).font(BOLD).into());
        }
        let (label, moves) = self.next_label(next.is_some());
        section.push(
            pill(
                p,
                &label,
                Tone::Selected,
                PILL,
                moves.then_some(op(Op::Next)),
            )
            .width(Fill)
            .into(),
        );
        section
    }

    /// The plan's current step, while the stage shows something that isn't
    /// it: a result shown on the fly in the middle of the plan. A live slot
    /// or a note has nothing of its own on stage, so it's never off.
    pub(crate) fn off_plan(&self) -> Option<Step> {
        let step = self.engine.0.current_step()?;
        let scene = self.stage.program.as_ref()?;
        match &step.action {
            Action::Show { resource } if resource.id != scene.resource.id => Some(step),
            Action::Blank => Some(step),
            _ => None,
        }
    }

    /// What PgDn does now, for its button, and whether it does anything:
    /// the next slide or page of what's on stage while it has one, and
    /// only then the plan's next step.
    pub(crate) fn next_label(&self, has_next_step: bool) -> (String, bool) {
        match self.stage.program.as_ref().filter(|s| s.has_next_frame()) {
            Some(scene) => (
                keys::with_key(&format!("Next {}", unit(&scene.resource)), Op::Next),
                true,
            ),
            None => (keys::with_key("Next step", Op::Next), has_next_step),
        }
    }

    /// A plan step as a monitor: its first slide or picture, or what kind
    /// of step it is.
    fn step_picture(&self, step: &Step) -> Element<'_, Message> {
        let p = self.palette;
        match &step.action {
            Action::Show { resource } => self.picture(
                resource
                    .location
                    .as_text()
                    .and_then(|body| self.slides.slides(body).first().cloned()),
                0,
                self.media.preloaded(&resource.id),
                self.thumbs.get(resource),
                self.loading_note(resource),
                Some(p.primary),
            ),
            Action::Live { hint, .. } => {
                match hint.as_ref().filter(|q| !q.text.trim().is_empty()) {
                    Some(query) => empty_monitor(
                        p,
                        &format!(
                            "A live slot: “{}” opens when you get there",
                            query.text.trim()
                        ),
                    ),
                    None => empty_monitor(p, "A live slot: search when you get there"),
                }
            }
            Action::Blank => empty_monitor(p, "A black screen"),
            Action::Note | Action::Section { .. } => {
                empty_monitor(p, "A note for you, nothing on stage")
            }
        }
    }

    /// The selected plan step, ready to show (a section stands for its
    /// first step).
    pub(crate) fn cued(&self) -> Option<Step> {
        let plan = self.engine.0.plan();
        let at = plan.position(self.selected?)?;
        plan.steps()[at..].iter().find(|s| !s.is_section()).cloned()
    }

    /// A picture of a scene: its image or page, its current slide, or a label.
    pub(crate) fn monitor<'a>(
        &'a self,
        scene: &'a Scene,
        content: Option<&'a Content>,
        ring: Option<Color>,
    ) -> Element<'a, Message> {
        if let Media::Failed { reason } = &scene.media {
            return framed(
                text(format!("Can't show this: {reason}"))
                    .size(BODY)
                    .color(self.palette.error),
                ring,
            );
        }
        let loading = self.loading_note(&scene.resource);
        self.picture(
            self.slides.slide(scene),
            scene.frame(),
            content,
            self.thumbs.get(&scene.resource),
            loading,
            ring,
        )
    }

    /// What a monitor shows: the slide's text, the loaded image or page, a
    /// video's `still` (its thumbnail) marked as a video, or `loading`
    /// until something has loaded, over the still when there is one.
    fn picture<'a>(
        &'a self,
        slide: Option<String>,
        frame: usize,
        content: Option<&'a Content>,
        still: Option<image::Handle>,
        loading: String,
        ring: Option<Color>,
    ) -> Element<'a, Message> {
        let p = self.palette;
        // A still with a few words in its corner: which video this is
        // matters more than that it is one.
        let marked = |still: &image::Handle, icon: Option<Icon>, words: String| {
            let mut mark = row![].spacing(4).align_y(Alignment::Center);
            if let Some(icon) = icon {
                mark = mark.push(look::icon(icon, 14.0, Color::WHITE));
            }
            // White on a dark scrim: it reads over any still.
            let mark = container(
                mark.push(
                    text(words)
                        .size(META)
                        .color(Color::WHITE)
                        .wrapping(text::Wrapping::None),
                ),
            )
            .padding([3, 8])
            .style(|_| container::Style {
                background: Some(Color::BLACK.scale_alpha(0.7).into()),
                border: iced::Border {
                    radius: look::PILL_RADIUS.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            });
            framed_with(
                stack![
                    thumb(still),
                    container(mark)
                        .align_left(Fill)
                        .align_bottom(Fill)
                        .padding(8),
                ],
                ring,
                2,
            )
        };
        let picture: Element<'a, Message> = match (slide, content) {
            (None, Some(Content::Video { .. })) if still.is_some() => {
                return marked(still.as_ref().unwrap(), Some(Icon::Play), "video".into());
            }
            (None, None) if still.is_some() => {
                return marked(still.as_ref().unwrap(), None, loading);
            }
            // A slide is a miniature of the stage, edge to edge.
            (Some(body), _) => {
                let look = &self.look;
                return framed_with(
                    crate::backdrop(look, fit::fit_text(body, look), crate::Over::Text),
                    ring,
                    2,
                );
            }
            (None, Some(Content::Image(handle))) => thumb(handle),
            (None, Some(Content::Pages(pages))) => thumb(&pages[frame.min(pages.len() - 1)]),
            (None, Some(Content::Video { .. })) => row![
                look::icon(Icon::Play, 18.0, p.on_surface_variant),
                text("video").size(BODY).color(p.on_surface_variant),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
            (None, None) => text(loading).size(BODY).color(p.on_surface_variant).into(),
        };
        framed(picture, ring)
    }

    fn loading_note(&self, resource: &Resource) -> String {
        self.download_state(resource)
            .unwrap_or_else(|| "loading…".into())
    }

    /// A YouTube video's download state while it's worth saying ("queued",
    /// "downloading 42%", why it failed), or `None`. Ready is how things
    /// should be, so it goes without saying.
    pub(crate) fn download_state(&self, resource: &Resource) -> Option<String> {
        let video = YoutubeVideo::of(resource)?;
        self.downloads
            .0
            .state(&video.id)
            .filter(|state| !matches!(state, crate::youtube::Download::Ready(_)))
            .map(|state| state.to_string())
    }

    /// The scene's title, and one line of what it is and where in it the
    /// stage is: "Juan 3:16-17 / RV1909 · slide 1 of 2".
    fn caption<'a>(&self, scene: &'a Scene) -> Element<'a, Message> {
        let p = self.palette;
        let resource = &scene.resource;
        let mut facts: Vec<String> = resource.subtitle.iter().cloned().collect();
        match &scene.media {
            Media::Frames { frame, count } => facts.extend(match count {
                Some(count) if count.get() == 1 => None,
                Some(count) => Some(format!("{} {} of {count}", unit(resource), frame + 1)),
                None => Some(format!("{} {}", unit(resource), frame + 1)),
            }),
            Media::Timed(_) => {}
            Media::Failed { .. } => facts.push("failed".into()),
        }
        if let Some(download) = self.download_state(resource) {
            facts.push(download);
        }
        column![
            single_line(&resource.title).size(BODY).font(BOLD),
            single_line(facts.join(" · "))
                .size(META)
                .color(p.on_surface_variant),
        ]
        .spacing(2)
        .into()
    }

    fn transport(&self, timed: &Timed) -> Element<'_, Message> {
        let p = self.palette;
        let icon = match timed.playback {
            Playback::Playing => Icon::Pause,
            Playback::Paused => Icon::Play,
            Playback::Ended => Icon::Replay,
        };
        let play = button(
            container(look::icon(icon, 18.0, p.on_primary))
                .center_x(Fill)
                .center_y(Fill),
        )
        .width(PILL)
        .height(PILL)
        .on_press(op(Op::PlayPause))
        .style(look::pill(p, Tone::Solid));
        let position = self.scrub.unwrap_or(timed.position.as_secs_f64());
        let timeline: Element<'_, Message> = match timed.duration {
            Some(duration) if !duration.is_zero() => {
                slider(0.0..=duration.as_secs_f64(), position, Message::Scrub)
                    .step(0.1)
                    .on_release(Message::Scrubbed)
                    .style(look::timeline(p))
                    .width(Fill)
                    .into()
            }
            _ => Space::new().width(Fill).into(),
        };
        let times = format!(
            "{} / {}",
            length::clock(Duration::from_secs_f64(position)),
            timed.duration.map_or("?".into(), length::clock)
        );
        row![
            play,
            timeline,
            text(times).size(META).color(p.on_surface_variant)
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
    }
}

/// The middle pane's content, at the launcher's width.
pub(crate) fn op(op: Op) -> Message {
    Message::Key {
        op,
        only_if_empty: false,
    }
}

/// "slide" for text, "page" for documents, "frame" for anything else.
/// The tint for an issue: amber for what may sort itself out, red for what
/// won't, plain when ready.
pub(crate) fn level_tone(level: Option<Level>) -> Tone {
    match level {
        None => Tone::Plain,
        Some(Level::Waiting) => Tone::Live,
        Some(Level::Problem) => Tone::Danger,
    }
}

impl App {
    /// The slides (verses, stanzas, pages) of what's on stage, the current
    /// one marked; clicking one shows it. A passage's slides are named by
    /// its verses. Nothing for a scene of one slide or a timeline.
    fn slide_strip<'a>(&self, scene: &Scene) -> Option<Element<'a, Message>> {
        let p = self.palette;
        let Media::Frames {
            frame,
            count: Some(count),
        } = &scene.media
        else {
            return None;
        };
        let count = count.get();
        if count < 2 {
            return None;
        }
        let verses: Vec<String> = scene
            .resource
            .meta
            .get("verses")
            .map(|list| list.split(',').map(|v| v.trim().to_string()).collect())
            .unwrap_or_default();
        let mut labels = match scene.resource.location.as_text() {
            Some(body) => self.slides.labels(body, &verses),
            None => Vec::new(),
        };
        if labels.len() != count {
            labels = (1..=count).map(|n| n.to_string()).collect();
        }
        let chips = labels.into_iter().enumerate().map(|(n, label)| {
            let tone = if n == *frame {
                Tone::Selected
            } else {
                Tone::Plain
            };
            button(
                text(label)
                    .size(META)
                    .font(if n == *frame { BOLD } else { MEDIUM })
                    .color(if n == *frame {
                        p.primary
                    } else {
                        p.on_surface_variant
                    }),
            )
            .padding([4, 9])
            .on_press(Message::GotoFrame(n))
            .style(look::row(p, tone))
            .into()
        });
        let strip = row(chips).spacing(4).wrap();
        Some(
            container(scrollable(strip).spacing(4))
                .max_height(STRIP_HEIGHT)
                .into(),
        )
    }
}

/// How tall the slide strip grows before it scrolls: about three rows.
const STRIP_HEIGHT: f32 = 96.0;

pub(crate) fn unit(resource: &Resource) -> &'static str {
    match resource.kind {
        lantern_engine::Kind::Text => "slide",
        lantern_engine::Kind::Document | lantern_engine::Kind::Presentation => "page",
        _ => "frame",
    }
}

/// A list that scrolls, with its scrollbar beside the rows instead of over them.
pub(crate) fn scrolling<'a>(
    content: impl Into<Element<'a, Message>>,
) -> scrollable::Scrollable<'a, Message> {
    scrollable(content).spacing(6).height(Fill)
}

/// One line of text; what doesn't fit is cut at the row's edge.
pub(crate) fn single_line(content: impl Into<String>) -> crate::cut::Cut {
    crate::cut::cut(content)
}

/// How a text starts, on one line, for a row's second line: its first
/// words, up to about a row's width. `None` for an empty text.
pub(crate) fn opening(body: &str) -> Option<String> {
    const WORDS: usize = 90;
    let line = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    Some(match line.char_indices().nth(WORDS) {
        Some((end, _)) => format!("{}…", line[..end].trim_end()),
        None => line,
    })
}

/// A section heading: bold `primary`.
fn heading(p: Palette, label: &str) -> Element<'_, Message> {
    text(label).size(HEADING).font(BOLD).color(p.primary).into()
}

pub(crate) fn chip<'a>(p: Palette, label: &str, tone: Tone) -> Element<'a, Message> {
    container(text(label.to_string()).size(META).font(BOLD))
        .padding([3, 10])
        .style(look::chip(p, tone))
        .into()
}

/// A source's name over its rows, with how many there are.
fn group_heading(p: Palette, name: &str, rows: usize) -> Element<'static, Message> {
    container(
        text(format!("{name} · {rows}"))
            .size(META)
            .color(p.on_surface_variant),
    )
    .padding(iced::Padding {
        top: 10.0,
        right: 12.0,
        bottom: 4.0,
        left: 12.0,
    })
    .into()
}

/// `!b   RV1909, KJV`. Clicking it starts the bang in the search box.
fn bang_row(p: Palette, bang: Bang) -> Element<'static, Message> {
    let mut line = row![
        text(format!("!{}", bang.bang))
            .size(BODY)
            .font(BOLD)
            .color(p.primary)
            .width(48),
        container(single_line(bang.sources.join(", ")).size(BODY)).width(Fill),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if bang.on_request {
        line = line.push(
            text(format!("{} to search", keys::label(Op::Enter)))
                .size(META)
                .color(p.on_surface_variant)
                .wrapping(text::Wrapping::None),
        );
    }
    button(line)
        .width(Fill)
        .padding([12, 12])
        .on_press(Message::Bang(bang.bang))
        .style(look::row(p, Tone::Plain))
        .into()
}

/// A black 16:9-ish monitor with `content` centred, ringed in `ring`.
fn framed<'a>(
    content: impl Into<Element<'a, Message>>,
    ring: Option<Color>,
) -> Element<'a, Message> {
    framed_with(content, ring, 12)
}

/// A monitor, its content `padding` inside the ring.
fn framed_with<'a>(
    content: impl Into<Element<'a, Message>>,
    ring: Option<Color>,
    padding: u16,
) -> Element<'a, Message> {
    // As tall as its slot in the stage pane.
    container(content)
        .center_x(Fill)
        .center_y(Fill)
        .padding(padding)
        .clip(true)
        .style(look::monitor(ring))
        .into()
}

pub(crate) fn empty_monitor<'a>(p: Palette, note: &str) -> Element<'a, Message> {
    framed(
        text(note.to_string())
            .size(META)
            .color(p.outline)
            .align_x(iced::alignment::Horizontal::Center),
        None,
    )
}

fn thumb(handle: &image::Handle) -> Element<'static, Message> {
    image(handle.clone())
        .width(Fill)
        .height(Fill)
        .content_fit(iced::ContentFit::Contain)
        .into()
}

pub(crate) fn pill<'a>(
    p: Palette,
    label: &str,
    tone: Tone,
    height: f32,
    message: Option<Message>,
) -> button::Button<'a, Message> {
    let weight = if tone == Tone::Plain { MEDIUM } else { BOLD };
    button(
        container(text(label.to_string()).size(BODY).font(weight))
            .center_x(Length::Shrink)
            .center_y(Fill),
    )
    .height(height)
    .padding([0, 16])
    .on_press_maybe(message)
    .style(look::pill(p, tone))
}

pub(crate) fn wide(button: button::Button<'_, Message>) -> button::Button<'_, Message> {
    button.width(Fill)
}

#[cfg(test)]
mod tests {
    use super::opening;

    #[test]
    fn a_passage_says_how_it_starts_on_one_line() {
        assert_eq!(opening("  \n "), None);
        assert_eq!(
            opening("EN el principio\nera el Verbo").as_deref(),
            Some("EN el principio era el Verbo")
        );
        let long = "amó ".repeat(40);
        let cut = opening(&long).unwrap();
        assert!(cut.ends_with('…') && cut.chars().count() <= 91, "{cut}");
    }

    #[test]
    fn the_side_panes_narrow_together_and_stop_at_their_least() {
        use super::pane_widths;
        assert_eq!(pane_widths(1920.0), (340.0, 280.0));
        assert_eq!(
            pane_widths(1180.0),
            (340.0, 280.0),
            "the live picture has its room"
        );
        let (plan, side) = pane_widths(1100.0);
        assert!(plan < 340.0 && side < 280.0, "{plan} {side}");
        assert_eq!(
            pane_widths(300.0),
            (280.0, 220.0),
            "never below their least"
        );
    }
}
