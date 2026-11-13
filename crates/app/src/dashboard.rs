//! The operator's window: three panes joined by hairlines. The plan on the
//! left, the launcher (search box and results) in the middle, and the stage
//! on the right: what the audience sees, and below it the preview (live
//! mode) or the plan's next step (plan mode).

use std::time::{Duration, SystemTime};

use iced::widget::{
    Space, button, column, container, image, pin, row, scrollable, slider, stack, text, text_input,
};
use iced::{Alignment, Color, Element, Fill, Length};
use lantern_engine::{Action, Bang, Media, Playback, Resource, Scene, Scope, Step, Timed, Timing};

use crate::fit;
use crate::keys::Op;
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

const STAGE_WIDTH: f32 = 360.0;
const LAUNCHER_WIDTH: f32 = 720.0;
const MONITOR_HEIGHT: f32 = 184.0;
/// How wide a result's thumbnail is; a plan row's is smaller.
const RESULT_THUMB: f32 = 96.0;
pub(crate) const STEP_THUMB: f32 = 64.0;

/// The keys worth knowing, under the results.
const HINTS: &str = "PgDn next step · Enter look, again to show · double-click shows · Ctrl+A add to the plan · \
                     ← → slides · Ctrl+B blackout · Esc clear";

impl App {
    pub(crate) fn dashboard_view(&self) -> Element<'_, Message> {
        let p = self.palette;
        let seam = || {
            container(Space::new())
                .width(1)
                .height(Fill)
                .style(look::hairline(p))
        };
        let timing = self.timing();
        let panes = row![
            self.plan_pane(&timing),
            seam(),
            self.launcher(&timing),
            seam(),
            self.stage_pane(),
        ]
        .height(Fill);
        let window = container(panes)
            .width(Fill)
            .height(Fill)
            .style(look::window(p));
        // A moving drag's card follows the pointer, just off to its side so
        // the rows under the pointer still know it's there.
        match self.drag.as_ref().filter(|drag| drag.moving) {
            Some(drag) => {
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
                stack![window, pin(card).x(drag.at.x + 16.0).y(drag.at.y + 8.0)].into()
            }
            None => window.into(),
        }
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

    fn launcher(&self, timing: &Timing) -> Element<'_, Message> {
        // The settings take the launcher's place while they're open.
        if let Some(draft) = &self.settings {
            return centered(self.settings_view(draft));
        }
        let p = self.palette;
        let mut header = row![
            look::icon(Icon::Lantern, 20.0, p.primary),
            text("Lantern").size(HEADING).font(BOLD).color(p.primary),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .height(36);
        // The running section and when the service should end.
        match (timing.running(), timing.end) {
            (Some(section), end) => {
                let (mut status, tone) = section_status(section);
                if let Some(end) = end {
                    let left = end.duration_since(SystemTime::now()).unwrap_or_default();
                    status = format!(
                        "{status} · ends in {}",
                        length::rough(left).trim_start_matches('~')
                    );
                }
                header = header
                    .push(chip(p, &status, tone))
                    .push(Space::new().width(Fill));
            }
            (None, _) => header = header.push(Space::new().width(Fill)),
        }
        let gear = button(look::icon(Icon::Settings, 18.0, p.on_surface_variant))
            .padding(8)
            .on_press(op(Op::Settings))
            .style(look::pill(p, Tone::Plain));
        header = header.push(gear);
        let mut body = column![header, self.search_box(), self.results()]
            .spacing(12)
            .height(Fill);
        for problem in self.problems() {
            body = body.push(text(problem).size(META).color(p.error));
        }
        body = body.push(text(HINTS).size(META).color(p.on_surface_variant));
        centered(body)
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
                _ => (
                    Space::new().into(),
                    "Search everything, ! for one source, or paste a link".into(),
                ),
            };
        let input = text_input(&placeholder, &self.search.query)
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

    /// A small picture of `resource`, `width` wide at 16:9: a text's first
    /// slide in the stage's look, a picture, page or video still once it's
    /// made, and the kind's icon until then (or when there's none).
    pub(crate) fn thumbnail<'a>(&self, resource: &Resource, width: f32) -> Element<'a, Message> {
        let p = self.palette;
        let height = width * 9.0 / 16.0;
        let look = &self.look;
        let picture: Element<'a, Message> = match resource.location.as_text() {
            Some(body) => {
                let first = self
                    .slides
                    .slides(body)
                    .first()
                    .cloned()
                    .unwrap_or_default();
                crate::backdrop(look, fit::fit_text(first, look), crate::Over::Text)
            }
            None => match self.thumbs.get(resource) {
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
            },
        };
        container(picture)
            .width(width)
            .height(height)
            .clip(true)
            .style(look::thumb(p))
            .into()
    }

    // The stage pane

    fn stage_pane(&self) -> Element<'_, Message> {
        let p = self.palette;
        // What Enter or PgDn would show: the result or step being looked
        // at, else the plan's next step.
        let lower = if self.stage.preview.is_some() {
            self.preview_section()
        } else {
            self.next_section()
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
            // Without a program the projector still shows the look's image.
            (None, false) if self.look.image.is_some() => chip(p, "background", Tone::Plain),
            (None, false) => chip(p, "empty", Tone::Plain),
        };
        let mut section = column![
            row![heading(p, "On stage"), Space::new().width(Fill), state]
                .align_y(Alignment::Center)
        ]
        .spacing(10);
        section = section.push(match scene {
            _ if blackout => empty_monitor(p, "The projector is black. Ctrl+B brings it back."),
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
            section = section.push(self.caption(scene));
            if let Some(strip) = self.slide_strip(scene) {
                section = section.push(strip);
            }
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
        // A selected plan step with nothing to preview (a black screen, a
        // live slot) still lines up here, ready to show.
        let cued = self
            .cued()
            .filter(|step| !matches!(step.action, Action::Show { .. }));
        let mut section = column![heading(p, "Preview")].spacing(10);
        match (scene, &cued) {
            (Some(scene), _) => {
                let content = self.media.preloaded(&scene.resource.id);
                section = section
                    .push(self.monitor(scene, content, Some(p.primary)))
                    .push(self.caption(scene));
            }
            (None, Some(step)) => {
                section = section
                    .push(self.step_picture(step))
                    .push(text(step.label.clone()).size(BODY).font(BOLD));
            }
            (None, None) => {
                section = section.push(empty_monitor(p, "Pick a result to preview it here"))
            }
        }
        let has = scene.is_some() || cued.is_some();
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
        section = section.push(match &next {
            Some(step) => self.step_picture(step),
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
            let mark = container(mark.push(single_line(words).size(META).color(Color::WHITE)))
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

    /// A YouTube video's download state ("downloading 42%"), or `None`.
    pub(crate) fn download_state(&self, resource: &Resource) -> Option<String> {
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
fn centered<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(container(content).max_width(LAUNCHER_WIDTH))
        .center_x(Fill)
        .height(Fill)
        .padding([16, 24])
        .into()
}

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
pub(crate) fn single_line<'a>(content: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(content).wrapping(text::Wrapping::None)
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
    framed_with(content, ring, 12)
}

/// A monitor, its content `padding` inside the ring.
fn framed_with<'a>(
    content: impl Into<Element<'a, Message>>,
    ring: Option<Color>,
    padding: u16,
) -> Element<'a, Message> {
    container(content)
        .center_x(Fill)
        .height(MONITOR_HEIGHT)
        .center_y(MONITOR_HEIGHT)
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
