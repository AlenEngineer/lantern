//! The operator's window: three panes joined by hairlines. The plan on the
//! left, the launcher (search box and results) in the middle, and the stage
//! on the right: what the audience sees, and below it the preview (live
//! mode) or the plan's next step (plan mode).

use std::time::{Duration, SystemTime};

use iced::widget::{
    Space, button, column, container, image, row, scrollable, slider, text, text_input,
};
use iced::{Alignment, Color, Element, Fill, Length};
use lantern_engine::{Action, Bang, Media, Plan, Playback, Resource, Scene, Scope, Step, Timed};

use crate::keys::Op;
use crate::look::{self, BODY, BOLD, HEADING, Icon, MEDIUM, META, PILL, PILL_SMALL, Tone};
use crate::palette::Palette;
use crate::plans::{self, Entry, Notice, Switch, file_name};
use crate::search::{Group, Live, supported};
use crate::stage::Content;
use crate::youtube::YoutubeVideo;
use crate::{App, Message};

pub const SEARCH_BOX: &str = "search";
pub const RESULTS: &str = "results";
pub const NAME_BOX: &str = "plan-name";

const PLAN_WIDTH: f32 = 280.0;
const STAGE_WIDTH: f32 = 360.0;
const LAUNCHER_WIDTH: f32 = 720.0;
const MONITOR_HEIGHT: f32 = 184.0;

/// What the operator is doing, shown in the launcher's header. Searching
/// switches to live, stepping through the plan to plan, and Ctrl+P either way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    /// Finding things and showing them on the fly: the stage pane shows the
    /// preview.
    #[default]
    Live,
    /// Following the plan: the stage pane shows the next step.
    Plan,
}

impl Mode {
    pub fn toggled(self) -> Mode {
        match self {
            Mode::Live => Mode::Plan,
            Mode::Plan => Mode::Live,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Mode::Live => "live",
            Mode::Plan => "plan",
        }
    }

    fn tone(self) -> Tone {
        match self {
            Mode::Live => Tone::Live,
            Mode::Plan => Tone::Selected,
        }
    }

    /// The keys worth knowing in this mode.
    fn hints(self) -> &'static str {
        match self {
            Mode::Live => {
                "Enter preview, again to show · ↑↓ pick · Ctrl+A add to plan · Tab bang · \
                 Esc clear · Ctrl+P plan mode"
            }
            Mode::Plan => {
                "PgDn next step · PgUp back · ← → slides · Ctrl+Space play · Ctrl+B blackout · \
                 Ctrl+P live mode"
            }
        }
    }
}

impl App {
    pub(crate) fn dashboard_view(&self) -> Element<'_, Message> {
        let p = self.palette;
        let seam = || {
            container(Space::new())
                .width(1)
                .height(Fill)
                .style(look::hairline(p))
        };
        let panes = row![
            self.plan_pane(),
            seam(),
            self.launcher(),
            seam(),
            self.stage_pane(),
        ]
        .height(Fill);
        container(panes)
            .width(Fill)
            .height(Fill)
            .style(look::window(p))
            .into()
    }

    // The launcher

    fn launcher(&self) -> Element<'_, Message> {
        let p = self.palette;
        let other = self.mode.toggled().name();
        let header = row![
            look::icon(Icon::Lantern, 20.0, p.primary),
            text("Lantern").size(HEADING).font(BOLD).color(p.primary),
            chip(p, self.mode.name(), self.mode.tone()),
            Space::new().width(Fill),
            text(format!("Ctrl+P {other} mode"))
                .size(META)
                .color(p.on_surface_variant),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .height(36);
        let mut body = column![header, self.search_box(), self.results()]
            .spacing(12)
            .height(Fill);
        for problem in self.problems() {
            body = body.push(text(problem).size(META).color(p.error));
        }
        body = body.push(
            text(self.mode.hints())
                .size(META)
                .color(p.on_surface_variant),
        );
        container(container(body).max_width(LAUNCHER_WIDTH))
            .center_x(Fill)
            .height(Fill)
            .padding([16, 24])
            .into()
    }

    /// The config, the index and the colours, only when something is wrong
    /// or still loading.
    fn problems(&self) -> Vec<String> {
        let mut problems: Vec<String> = self.config_status.problem().into_iter().collect();
        problems.extend(self.library.0.problems());
        problems.extend(self.palette_error.clone());
        problems
    }

    fn search_box(&self) -> Element<'_, Message> {
        let p = self.palette;
        let scope: Element<'_, Message> = match &self.search.scope {
            Scope::Everything => Space::new().into(),
            Scope::Bang(bang) => chip(p, &bang.sources.join(", "), Tone::Plain),
            Scope::Unknown(bang) => chip(p, &format!("!{bang}?"), Tone::Danger),
        };
        let input = text_input(
            "Search everything, ! for one source, or paste a link",
            &self.search.query,
        )
        .id(SEARCH_BOX)
        .on_input(Message::Query)
        .on_submit(Message::Key {
            op: Op::Enter,
            only_if_empty: false,
        })
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

    fn results(&self) -> Element<'_, Message> {
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
        if search.query.trim().is_empty() {
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
                "Press Enter to search {}.",
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
            icon_disc(p, Icon::of(&resource.kind), tone),
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
            .on_press_maybe(ok.then(|| Message::Pick(resource.id.clone())))
            .style(look::row(p, tone))
            .into()
    }

    // The stage pane

    fn stage_pane(&self) -> Element<'_, Message> {
        let p = self.palette;
        let lower = match self.mode {
            Mode::Live => self.preview_section(),
            Mode::Plan => self.next_section(),
        };
        column![
            self.on_stage_section(),
            container(Space::new())
                .height(1)
                .width(Fill)
                .style(look::hairline(p)),
            lower,
        ]
        .spacing(12)
        .padding(16)
        .width(STAGE_WIDTH)
        .height(Fill)
        .into()
    }

    fn on_stage_section(&self) -> Element<'_, Message> {
        let p = self.palette;
        let scene = self.stage.program.as_ref();
        let blackout = self.stage.blackout;
        let state = match (scene, blackout) {
            (_, true) => chip(p, "blackout", Tone::Danger),
            (Some(_), false) => chip(p, "● live", Tone::Live),
            (None, false) => chip(p, "empty", Tone::Plain),
        };
        let mut section = column![
            row![heading(p, "On stage"), Space::new().width(Fill), state]
                .align_y(Alignment::Center)
        ]
        .spacing(10);
        section = section.push(match scene {
            Some(_) if blackout => {
                empty_monitor(p, "The projector is black. Ctrl+B brings it back.")
            }
            Some(scene) => self.monitor(scene, self.media.content(scene.id), Some(p.tertiary)),
            None => empty_monitor(p, "Nothing on the projector"),
        });
        if let Some(scene) = scene {
            section = section.push(self.caption(scene));
            if let Some(timed) = scene.timed() {
                section = section.push(self.transport(timed));
            }
        }
        let blackout_label = if blackout {
            "Blackout on · Ctrl+B"
        } else {
            "Blackout · Ctrl+B"
        };
        section
            .push(
                row![
                    wide(pill(
                        p,
                        blackout_label,
                        Tone::Danger,
                        PILL,
                        Some(Message::Key {
                            op: Op::Blackout,
                            only_if_empty: false,
                        })
                    )),
                    pill(
                        p,
                        "Clear",
                        Tone::Plain,
                        PILL,
                        scene.is_some().then_some(Message::Key {
                            op: Op::Clear,
                            only_if_empty: false,
                        })
                    ),
                ]
                .spacing(4),
            )
            .into()
    }

    fn preview_section(&self) -> Element<'_, Message> {
        let p = self.palette;
        let scene = self.stage.preview.as_ref();
        let mut section = column![heading(p, "Preview")].spacing(10);
        match scene {
            Some(scene) => {
                let content = self.media.preloaded(&scene.resource.id);
                section = section
                    .push(self.monitor(scene, content, Some(p.primary)))
                    .push(self.caption(scene));
            }
            None => section = section.push(empty_monitor(p, "Pick a result to preview it here")),
        }
        let has = scene.is_some();
        section
            .push(
                row![
                    wide(pill(
                        p,
                        "Show now · Enter",
                        Tone::Selected,
                        PILL,
                        has.then_some(op(Op::Take))
                    )),
                    wide(pill(
                        p,
                        "Add · Ctrl+A",
                        Tone::Plain,
                        PILL,
                        has.then_some(op(Op::AddToPlan))
                    )),
                ]
                .spacing(4),
            )
            .into()
    }

    fn next_section(&self) -> Element<'_, Message> {
        let p = self.palette;
        let next = self.engine.0.up_next();
        let mut section = column![heading(p, "Up next")].spacing(10);
        section = section.push(match next.as_ref().map(|step| &step.action) {
            Some(Action::Show { resource }) => self.picture(
                resource
                    .location
                    .as_text()
                    .and_then(|body| body.split("\n\n").next())
                    .map(str::to_string),
                0,
                self.media.preloaded(&resource.id),
                self.loading_note(resource),
                Some(p.primary),
            ),
            Some(Action::Live { .. }) => empty_monitor(p, "A live slot: search when you get there"),
            Some(Action::Blank) => empty_monitor(p, "A black screen"),
            Some(Action::Note) => empty_monitor(p, "A note for you, nothing on stage"),
            None => empty_monitor(p, "The end of the plan"),
        });
        if let Some(step) = &next {
            section = section.push(text(step.label.clone()).size(BODY).font(BOLD));
        }
        section
            .push(
                pill(
                    p,
                    "Next step · PgDn",
                    Tone::Selected,
                    PILL,
                    next.is_some().then_some(op(Op::Next)),
                )
                .width(Fill),
            )
            .into()
    }

    /// A picture of a scene: its image or page, its current slide, or a label.
    fn monitor<'a>(
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
        self.picture(scene.text(), scene.frame(), content, loading, ring)
    }

    /// What a monitor shows: the slide's text, the loaded image or page,
    /// that a video is ready, or `loading` until something has loaded.
    fn picture<'a>(
        &'a self,
        slide: Option<String>,
        frame: usize,
        content: Option<&'a Content>,
        loading: String,
        ring: Option<Color>,
    ) -> Element<'a, Message> {
        let p = self.palette;
        let picture: Element<'a, Message> = match (slide, content) {
            (Some(body), _) => text(body)
                .size(BODY)
                .color(Color::WHITE)
                .align_x(iced::alignment::Horizontal::Center)
                .into(),
            (None, Some(Content::Image(handle))) => thumb(handle),
            (None, Some(Content::Pages(pages))) => thumb(&pages[frame.min(pages.len() - 1)]),
            (None, Some(Content::Video { stream, .. })) => row![
                look::icon(Icon::Play, 18.0, p.on_surface_variant),
                text(if *stream { "streaming" } else { "video" })
                    .size(BODY)
                    .color(p.on_surface_variant),
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

    /// A YouTube video's download state ("downloading 42%"), or `None`.
    fn download_state(&self, resource: &Resource) -> Option<String> {
        let video = YoutubeVideo::of(resource)?;
        self.downloads
            .0
            .state(&video.id)
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
        let stream = matches!(
            self.media.content(scene.id),
            Some(Content::Video { stream: true, .. })
        );
        if stream {
            facts.push("streaming".into());
        } else if let Some(download) = self.download_state(resource) {
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
            clock(Duration::from_secs_f64(position)),
            timed.duration.map_or("?".into(), clock)
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

    // The plan pane

    fn plan_pane(&self) -> Element<'_, Message> {
        let p = self.palette;
        let plan = self.engine.0.plan();
        let mut pane = column![self.plan_header(&plan)].spacing(8);
        if let Some(notice) = self.plan_notice(&plan) {
            pane = pane.push(notice);
        }
        pane = pane.push(match &self.plans.mode {
            plans::Mode::Steps | plans::Mode::Renaming(_) => self.steps(&plan),
            plans::Mode::Browsing(entries) => self.saved_plans(entries),
            plans::Mode::Confirm(switch) => confirm(p, &plan, switch),
        });
        let close = matches!(self.plans.mode, plans::Mode::Browsing(_));
        let buttons = if close {
            row![wide(pill(
                p,
                "Cancel",
                Tone::Plain,
                PILL_SMALL,
                Some(Message::ClosePlanMode)
            ))]
        } else {
            row![
                wide(pill(p, "Open", Tone::Plain, PILL_SMALL, Some(op(Op::Open)))),
                wide(pill(p, "Save", Tone::Plain, PILL_SMALL, Some(op(Op::Save)))),
                wide(pill(p, "New", Tone::Plain, PILL_SMALL, Some(op(Op::New)))),
            ]
        };
        pane.push(buttons.spacing(4))
            .padding([16, 12])
            .width(PLAN_WIDTH)
            .height(Fill)
            .into()
    }

    /// The plan's name (click to rename), an unsaved marker, and where the
    /// service is: "3 of 7".
    fn plan_header(&self, plan: &Plan) -> Element<'_, Message> {
        let p = self.palette;
        let name: Element<'_, Message> = match &self.plans.mode {
            plans::Mode::Renaming(draft) => text_input("Plan name", draft)
                .id(NAME_BOX)
                .on_input(Message::PlanName)
                .on_submit(Message::SubmitPlanName)
                .size(BODY)
                .padding([6, 10])
                .style(look::input(p))
                .into(),
            _ => button(
                single_line(plan.title.clone())
                    .size(HEADING)
                    .font(BOLD)
                    .color(p.primary),
            )
            .padding([4, 8])
            .on_press(Message::EditPlanName)
            .style(look::row(p, Tone::Plain))
            .width(Fill)
            .into(),
        };
        let at = self
            .engine
            .0
            .current_step()
            .and_then(|current| plan.steps().iter().position(|s| s.id == current.id));
        let mut place = match at {
            Some(at) => format!("{} of {}", at + 1, plan.len()),
            None => count(plan.len(), "step"),
        };
        if self.plans.is_dirty(plan) {
            place.push_str(" · unsaved");
        }
        row![name, text(place).size(META).color(p.on_surface_variant)]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
    }

    /// The last save or open: its error, or that it worked while nothing
    /// has changed since.
    fn plan_notice(&self, plan: &Plan) -> Option<Element<'_, Message>> {
        let p = self.palette;
        let (message, color) = match self.plans.notice.as_ref()? {
            Notice::Failed(why) => (why, p.error),
            Notice::Done(_) if self.plans.is_dirty(plan) => return None,
            Notice::Done(what) => (what, p.on_surface_variant),
        };
        Some(
            container(text(message.as_str()).size(META).color(color))
                .padding([0, 8])
                .into(),
        )
    }

    fn steps(&self, plan: &Plan) -> Element<'_, Message> {
        let p = self.palette;
        let current = self.engine.0.current_step().map(|step| step.id);
        let next = self.engine.0.up_next().map(|step| step.id);
        let at = current.and_then(|id| plan.steps().iter().position(|s| s.id == id));
        let mut steps = column![].spacing(2);
        for (index, step) in plan.steps().iter().enumerate() {
            let place = if Some(step.id) == current {
                Place::Current
            } else if Some(step.id) == next {
                Place::Next
            } else if at.is_some_and(|at| index < at) {
                Place::Done
            } else {
                Place::Later
            };
            steps = steps.push(self.step_row(index, step, place));
        }
        if plan.is_empty() {
            steps = steps.push(
                container(
                    text("The plan is empty. Preview something and press Ctrl+A to add it.")
                        .size(BODY)
                        .color(p.on_surface_variant),
                )
                .padding([4, 8]),
            );
        }
        scrolling(steps).into()
    }

    fn step_row(&self, index: usize, step: &Step, place: Place) -> Element<'_, Message> {
        let p = self.palette;
        // The current step is on stage unless something off the plan was
        // shown since.
        let shown = match &step.action {
            Action::Show { resource } => self
                .stage
                .program
                .as_ref()
                .is_some_and(|scene| scene.resource.id == resource.id),
            _ => false,
        };
        let tone = match (place, self.mode) {
            (Place::Current, _) if shown => Tone::Live,
            (Place::Next, Mode::Plan) => Tone::Selected,
            _ => Tone::Plain,
        };
        let (marker, note) = match place {
            Place::Current if shown => ("●".to_string(), Some("on stage".to_string())),
            Place::Current => ("●".to_string(), Some("current step".to_string())),
            Place::Next => ("○".to_string(), Some("next, PgDn".to_string())),
            Place::Done | Place::Later => ((index + 1).to_string(), None),
        };
        let label_color = match (place, tone) {
            (Place::Done, _) => p.outline,
            (_, Tone::Plain) => p.on_surface,
            (_, tone) => tone.text(p),
        };
        let mut label = column![
            single_line(step.label.clone())
                .size(BODY)
                .font(if tone == Tone::Plain { MEDIUM } else { BOLD })
                .color(label_color)
        ];
        let detail = note.or_else(|| match &step.action {
            Action::Show { resource } => self.download_state(resource),
            Action::Live { .. } => Some("live search".into()),
            Action::Blank => Some("black screen".into()),
            Action::Note => Some("note".into()),
        });
        if let Some(detail) = detail {
            label = label.push(single_line(detail).size(META).color(p.on_surface_variant));
        }
        let remove = button(text("✕").size(META).color(p.on_surface_variant))
            .padding([2, 8])
            .on_press(Message::Remove(step.id))
            .style(look::row(p, Tone::Plain));
        let line = row![
            text(marker).size(META).color(label_color).width(16),
            container(label).width(Fill).clip(true),
            remove,
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        button(line)
            .width(Fill)
            .padding([8, 10])
            .on_press(Message::GoTo(step.id))
            .style(look::row(p, tone))
            .into()
    }

    fn saved_plans<'a>(&self, entries: &'a [Entry]) -> Element<'a, Message> {
        let p = self.palette;
        let folder = self.plans.store().dir().display().to_string();
        let mut list = column![
            container(
                column![
                    text("Open a saved plan")
                        .size(BODY)
                        .font(BOLD)
                        .color(p.primary),
                    single_line(folder).size(META).color(p.on_surface_variant),
                ]
                .spacing(2)
            )
            .padding([4, 8])
        ]
        .spacing(2);
        if entries.is_empty() {
            list = list.push(
                container(
                    text("No saved plans yet. Save makes one.")
                        .size(BODY)
                        .color(p.on_surface_variant),
                )
                .padding([4, 8]),
            );
        }
        let now = SystemTime::now();
        for entry in entries {
            let when = now.duration_since(entry.modified).unwrap_or_default();
            list = list.push(saved_plan(p, entry, ago(when)));
        }
        scrolling(list).into()
    }
}

/// Where a step is in the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Done,
    Current,
    Next,
    Later,
}

fn op(op: Op) -> Message {
    Message::Key {
        op,
        only_if_empty: false,
    }
}

/// "slide" for text, "page" for documents, "frame" for anything else.
fn unit(resource: &Resource) -> &'static str {
    match resource.kind {
        lantern_engine::Kind::Text => "slide",
        lantern_engine::Kind::Document | lantern_engine::Kind::Presentation => "page",
        _ => "frame",
    }
}

fn saved_plan(p: Palette, entry: &Entry, when: String) -> Element<'_, Message> {
    let (title, detail, color) = match &entry.summary {
        Ok((title, steps)) => (
            title.clone(),
            format!("{} · {when}", count(*steps, "step")),
            p.on_surface,
        ),
        Err(why) => (
            file_name(&entry.path),
            format!("can't read: {why}"),
            p.error,
        ),
    };
    button(
        column![
            single_line(title).size(BODY).font(MEDIUM).color(color),
            single_line(detail).size(META).color(p.on_surface_variant),
        ]
        .spacing(2),
    )
    .width(Fill)
    .padding([8, 10])
    .on_press_maybe(
        entry
            .summary
            .is_ok()
            .then(|| Message::OpenPlan(entry.path.clone())),
    )
    .style(look::row(p, Tone::Plain))
    .into()
}

fn confirm<'a>(p: Palette, plan: &Plan, switch: &Switch) -> Element<'a, Message> {
    let then = match switch {
        Switch::New => "start a new one",
        Switch::Open(_) => "open another",
    };
    column![
        text(format!(
            "“{}” was never saved. Discard it and {then}?",
            plan.title
        ))
        .size(BODY),
        text("Ctrl+S saves it instead.")
            .size(META)
            .color(p.on_surface_variant),
        row![
            wide(pill(
                p,
                "Discard",
                Tone::Danger,
                PILL_SMALL,
                Some(Message::DiscardPlan)
            )),
            wide(pill(
                p,
                "Keep it",
                Tone::Plain,
                PILL_SMALL,
                Some(Message::ClosePlanMode)
            )),
        ]
        .spacing(4),
    ]
    .spacing(10)
    .padding([4, 8])
    .height(Fill)
    .into()
}

/// "1 step", "3 steps".
fn count(n: usize, noun: &str) -> String {
    match n {
        1 => format!("1 {noun}"),
        n => format!("{n} {noun}s"),
    }
}

/// How long ago, roughly: "just now", "5 min ago", "3 h ago", "2 days ago".
fn ago(elapsed: Duration) -> String {
    let minutes = elapsed.as_secs() / 60;
    match minutes {
        0 => "just now".into(),
        1..60 => format!("{minutes} min ago"),
        60..1440 => format!("{} h ago", minutes / 60),
        1440..2880 => "1 day ago".into(),
        _ => format!("{} days ago", minutes / 1440),
    }
}

fn clock(d: Duration) -> String {
    format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60)
}

/// A list that scrolls, with its scrollbar beside the rows instead of over them.
fn scrolling<'a>(content: impl Into<Element<'a, Message>>) -> scrollable::Scrollable<'a, Message> {
    scrollable(content).spacing(6).height(Fill)
}

/// One line of text; what doesn't fit is cut at the row's edge.
fn single_line<'a>(content: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(content).wrapping(text::Wrapping::None)
}

/// A section heading: bold `primary`.
fn heading(p: Palette, label: &str) -> Element<'_, Message> {
    text(label).size(HEADING).font(BOLD).color(p.primary).into()
}

fn chip(p: Palette, label: &str, tone: Tone) -> Element<'static, Message> {
    container(text(label.to_string()).size(META).font(BOLD))
        .padding([3, 10])
        .style(look::chip(p, tone))
        .into()
}

/// A kind's icon in a small disc, tinted like its row.
fn icon_disc(p: Palette, icon: Icon, tone: Tone) -> Element<'static, Message> {
    let color = match tone {
        Tone::Plain => p.on_surface_variant,
        tone => tone.text(p),
    };
    container(look::icon(icon, 16.0, color))
        .width(32)
        .height(32)
        .center_x(32)
        .center_y(32)
        .style(look::chip(
            p,
            if tone == Tone::Plain {
                Tone::Plain
            } else {
                tone
            },
        ))
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
        text(bang.sources.join(", ")).size(BODY).width(Fill),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if bang.on_request {
        line = line.push(
            text("Enter to search")
                .size(META)
                .color(p.on_surface_variant),
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
    container(content)
        .center_x(Fill)
        .height(MONITOR_HEIGHT)
        .center_y(MONITOR_HEIGHT)
        .padding(12)
        .clip(true)
        .style(look::monitor(ring))
        .into()
}

fn empty_monitor(p: Palette, note: &str) -> Element<'static, Message> {
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

fn pill<'a>(
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

fn wide(button: button::Button<'_, Message>) -> button::Button<'_, Message> {
    button.width(Fill)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_are_rough() {
        let minutes = |m: u64| ago(Duration::from_secs(m * 60));
        assert_eq!(minutes(0), "just now");
        assert_eq!(minutes(5), "5 min ago");
        assert_eq!(minutes(130), "2 h ago");
        assert_eq!(minutes(1500), "1 day ago");
        assert_eq!(minutes(3 * 1440), "3 days ago");
    }

    #[test]
    fn modes_toggle_and_say_their_keys() {
        assert_eq!(Mode::Live.toggled(), Mode::Plan);
        assert_eq!(Mode::Plan.toggled(), Mode::Live);
        assert!(Mode::Live.hints().contains("Ctrl+P plan mode"));
        assert!(Mode::Plan.hints().contains("PgDn next step"));
    }
}
