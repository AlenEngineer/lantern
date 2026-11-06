//! The operator's window: search, preview, program, the plan and the
//! transport, plus the keys that drive them.

use std::time::Duration;

use chorquest_engine::{Action, Media, Playback, Resource, Scene, Step, Timed};
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{
    Space, button, column, container, image, row, scrollable, slider, text, text_input,
};
use iced::{Background, Border, Color, Element, Fill, Font, Length, Theme, event, font};

use crate::search::{Group, Live, supported};
use crate::stage::Content;
use crate::youtube::{Download, YoutubeVideo};
use crate::{App, Message};

pub const SEARCH_BOX: &str = "search";
pub const RESULTS: &str = "results";

/// What the operator can do from a key, a button or a script's `press`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    /// Enter: previews the first result if none is picked, otherwise takes.
    Enter,
    /// Puts the preview on program.
    Take,
    /// Adds the preview to the plan after the current step.
    AddToPlan,
    Next,
    Prev,
    PlayPause,
    Blackout,
    Clear,
    PickNext,
    PickPrev,
    FocusSearch,
}

enum Bind {
    Named(Named),
    Char(char),
}

/// Every key, by the name a script's `press` uses. While the search box has
/// focus it keeps the keys it uses for typing, so only Enter, Escape (which
/// leaves the box), Up, Down, Page Up and Page Down reach this table then.
const KEYS: &[(&str, Bind, Op)] = &[
    ("enter", Bind::Named(Named::Enter), Op::Enter),
    ("down", Bind::Named(Named::ArrowDown), Op::PickNext),
    ("up", Bind::Named(Named::ArrowUp), Op::PickPrev),
    // Presentation clickers send Page Down and Page Up.
    ("pagedown", Bind::Named(Named::PageDown), Op::Next),
    ("pageup", Bind::Named(Named::PageUp), Op::Prev),
    ("right", Bind::Named(Named::ArrowRight), Op::Next),
    ("left", Bind::Named(Named::ArrowLeft), Op::Prev),
    ("space", Bind::Named(Named::Space), Op::PlayPause),
    ("b", Bind::Char('b'), Op::Blackout),
    ("a", Bind::Char('a'), Op::AddToPlan),
    ("/", Bind::Char('/'), Op::FocusSearch),
];

const HELP: &str = "Enter preview, then take · ↑↓ pick · A add to plan · →/PgDn next · ←/PgUp back · \
                    Space play/pause · B blackout · Esc leaves the search box · / returns to it";

/// The op a script's `press NAME` stands for.
pub fn op_named(name: &str) -> Option<Op> {
    KEYS.iter()
        .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, _, op)| *op)
}

fn op_for(key: &Key) -> Option<Op> {
    KEYS.iter()
        .find(|(_, bind, _)| match (bind, key.as_ref()) {
            (Bind::Named(named), Key::Named(pressed)) => *named == pressed,
            (Bind::Named(Named::Space), Key::Character(" ")) => true,
            (Bind::Char(c), Key::Character(pressed)) => {
                pressed.chars().eq(std::iter::once(*c))
                    || pressed.to_lowercase().chars().eq(std::iter::once(*c))
            }
            _ => false,
        })
        .map(|(_, _, op)| *op)
}

/// Keys the focused search box didn't use. Held keys don't repeat, so
/// holding B can't flicker the projector.
pub fn key_event(
    event: iced::Event,
    status: event::Status,
    _: iced::window::Id,
) -> Option<Message> {
    let iced::Event::Keyboard(keyboard::Event::KeyPressed {
        key,
        modifiers,
        repeat,
        ..
    }) = event
    else {
        return None;
    };
    if status == event::Status::Captured || modifiers.command() || modifiers.alt() {
        return None;
    }
    let op = op_for(&key)?;
    let repeats = matches!(op, Op::PickNext | Op::PickPrev);
    (repeats || !repeat).then_some(Message::Op(op))
}

pub fn theme() -> Theme {
    Theme::custom(
        "chorquest",
        iced::theme::Palette {
            background: BG,
            text: TEXT,
            primary: BLUE,
            success: GREEN,
            warning: AMBER,
            danger: RED,
        },
    )
}

const BG: Color = rgb(0x0e, 0x10, 0x14);
const PANEL: Color = rgb(0x17, 0x1a, 0x20);
const RAISED: Color = rgb(0x22, 0x26, 0x2e);
const LINE: Color = rgb(0x2c, 0x31, 0x3a);
const TEXT: Color = rgb(0xe8, 0xea, 0xed);
const MUTED: Color = rgb(0x8b, 0x92, 0x9c);
const FAINT: Color = rgb(0x5a, 0x60, 0x6a);
const RED: Color = rgb(0xe5, 0x48, 0x4d);
const GREEN: Color = rgb(0x30, 0xa4, 0x6c);
const BLUE: Color = rgb(0x3e, 0x7b, 0xfa);
const AMBER: Color = rgb(0xf5, 0xa5, 0x24);

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

const BOLD: Font = Font {
    weight: font::Weight::Bold,
    ..Font::DEFAULT
};

impl App {
    pub(crate) fn dashboard_view(&self) -> Element<'_, Message> {
        let top = row![
            self.search_box(),
            self.blackout_button(),
            action("Clear", Message::Op(Op::Clear))
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);
        let monitors = row![self.preview_panel(), self.program_panel()]
            .spacing(12)
            .height(Length::FillPortion(5));
        let right = column![monitors, self.plan_panel().height(Length::FillPortion(4))].spacing(12);
        let body = row![
            self.results_panel().width(Length::FillPortion(2)),
            container(right).width(Length::FillPortion(3)),
        ]
        .spacing(12)
        .height(Fill);
        let mut status = vec![self.config_status.as_str().to_string()];
        status.extend(self.library.0.lines());
        let footer = column![
            text(HELP).size(13).color(MUTED),
            text(status.join(" · ")).size(12).color(FAINT),
        ]
        .spacing(2);
        container(column![top, body, footer].spacing(12).padding(12))
            .width(Fill)
            .height(Fill)
            .style(|_| container::Style::default().background(BG))
            .into()
    }

    fn search_box(&self) -> Element<'_, Message> {
        text_input(
            "Search files and Bibles, or paste a YouTube link",
            &self.search.query,
        )
        .id(SEARCH_BOX)
        .on_input(Message::Query)
        .on_submit(Message::Op(Op::Enter))
        .size(20)
        .padding(10)
        .width(Fill)
        .style(|theme: &Theme, status| text_input::Style {
            background: Background::Color(PANEL),
            border: Border {
                color: if matches!(status, text_input::Status::Focused { .. }) {
                    BLUE
                } else {
                    LINE
                },
                width: 1.5,
                radius: 8.0.into(),
            },
            ..text_input::default(theme, status)
        })
        .into()
    }

    fn blackout_button(&self) -> Element<'_, Message> {
        let on = self.stage.blackout;
        let label = if on { "BLACKOUT ON" } else { "Blackout" };
        button(text(label).size(15).font(BOLD).center())
            .width(150)
            .padding([10, 16])
            .on_press(Message::Op(Op::Blackout))
            .style(move |_, status| {
                if on {
                    filled(RED, TEXT, status)
                } else {
                    filled(RAISED, TEXT, status)
                }
            })
            .into()
    }

    fn results_panel(&self) -> container::Container<'_, Message> {
        let search = &self.search;
        let mut list = column![].spacing(2);
        for group in search.groups() {
            list = list.push(self.group_view(group));
        }
        let note = match (
            search.query.trim().is_empty(),
            search.groups().is_empty(),
            search.live,
        ) {
            (true, ..) => Some("Type to search. Results appear as you type.".to_string()),
            (false, _, Live::Waiting(_) | Live::Running(_)) => {
                Some("Checking links and Bible references…".to_string())
            }
            (false, true, Live::Idle) => Some(format!("Nothing found for “{}”.", search.query)),
            (false, false, Live::Idle) => None,
        };
        if let Some(note) = note {
            list = list.push(container(text(note).size(14).color(MUTED)).padding([6, 8]));
        }
        panel(
            column![
                heading("RESULTS", MUTED),
                scrollable(list).id(RESULTS).height(Fill),
            ]
            .spacing(8),
            LINE,
        )
    }

    fn group_view<'a>(&'a self, group: &'a Group) -> Element<'a, Message> {
        let mut rows = column![
            container(
                row![
                    text(group.name.to_uppercase())
                        .size(12)
                        .font(BOLD)
                        .color(MUTED),
                    text(group.hits.len().to_string()).size(12).color(FAINT),
                ]
                .spacing(6)
            )
            .padding(iced::Padding {
                top: 10.0,
                right: 8.0,
                bottom: 4.0,
                left: 8.0,
            })
        ]
        .spacing(2);
        if let Some(error) = &group.error {
            rows = rows.push(text(format!("failed: {error}")).size(13).color(RED));
        }
        for resource in &group.hits {
            rows = rows.push(self.result_row(resource));
        }
        rows.into()
    }

    fn result_row<'a>(&'a self, resource: &'a Resource) -> Element<'a, Message> {
        let ok = supported(resource);
        let picked = self.search.is_picked(resource);
        let kind = if ok {
            resource.kind.to_string()
        } else {
            format!("{} · not supported", resource.kind)
        };
        let mut line = column![
            row![
                text(&resource.title)
                    .size(16)
                    .color(if ok { TEXT } else { FAINT })
                    .width(Fill),
                text(format!("{kind}{}", self.download_note(resource)))
                    .size(12)
                    .color(if ok { MUTED } else { FAINT }),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
        ];
        if let Some(subtitle) = &resource.subtitle {
            line = line.push(
                text(subtitle)
                    .size(13)
                    .color(if ok { MUTED } else { FAINT }),
            );
        }
        button(line)
            .width(Fill)
            .padding([6, 8])
            .on_press_maybe(ok.then(|| Message::Pick(resource.id.clone())))
            .style(move |_, status| {
                let base = if picked {
                    GREEN.scale_alpha(0.22)
                } else {
                    Color::TRANSPARENT
                };
                let hover = matches!(status, button::Status::Hovered) && !picked;
                button::Style {
                    background: Some(Background::Color(if hover { RAISED } else { base })),
                    text_color: TEXT,
                    border: Border {
                        color: if picked { GREEN } else { Color::TRANSPARENT },
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..button::Style::default()
                }
            })
            .into()
    }

    fn preview_panel(&self) -> Element<'_, Message> {
        let scene = self.stage.preview.as_ref();
        let has = scene.is_some();
        let mut body = column![heading("PREVIEW", GREEN)].spacing(8);
        match scene {
            Some(scene) => {
                let content = self.media.preloaded(&scene.resource.id);
                body = body.push(self.monitor(scene, content));
                body = body.push(self.describe(scene));
            }
            None => {
                body = body.push(empty_monitor("Pick a result to preview it here"));
            }
        }
        body = body.push(
            row![
                wide(action_tinted(
                    "Take  ⏎",
                    GREEN,
                    has.then_some(Message::Op(Op::Take))
                )),
                wide(action_tinted(
                    "Add to plan  A",
                    RAISED,
                    has.then_some(Message::Op(Op::AddToPlan))
                )),
            ]
            .spacing(8),
        );
        panel(body, GREEN.scale_alpha(if has { 0.8 } else { 0.35 }))
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn program_panel(&self) -> Element<'_, Message> {
        let scene = self.stage.program.as_ref();
        let badge = match (scene, self.stage.blackout) {
            (_, true) => tag("BLACKOUT", RED),
            (Some(_), false) => tag("● LIVE", RED),
            (None, false) => tag("EMPTY", FAINT),
        };
        let mut body = column![
            row![heading("PROGRAM", RED), Space::new().width(Fill), badge]
                .align_y(iced::Alignment::Center)
        ]
        .spacing(8);
        match scene {
            Some(scene) => {
                body = body.push(if self.stage.blackout {
                    empty_monitor("The projector is black. B brings it back.")
                } else {
                    self.monitor(scene, self.media.content(scene.id))
                });
                body = body.push(self.describe(scene));
                if let Some(timed) = scene.timed() {
                    body = body.push(self.transport(timed));
                }
            }
            None => {
                body = body.push(empty_monitor("Nothing on the projector"));
            }
        }
        panel(
            body,
            RED.scale_alpha(if scene.is_some() { 0.9 } else { 0.35 }),
        )
        .width(Fill)
        .height(Fill)
        .into()
    }

    /// A small picture of a scene: the image or page, the current stanza, or a label.
    fn monitor<'a>(
        &'a self,
        scene: &'a Scene,
        content: Option<&'a Content>,
    ) -> Element<'a, Message> {
        let picture: Element<'a, Message> = match (&scene.media, scene.text(), content) {
            (Media::Failed { reason }, ..) => text(format!("Can't show this: {reason}"))
                .size(14)
                .color(RED)
                .into(),
            (_, Some(body), _) => text(body)
                .size(14)
                .color(TEXT)
                .align_x(iced::alignment::Horizontal::Center)
                .into(),
            (_, None, Some(Content::Image(handle))) => thumb(handle),
            (_, None, Some(Content::Pages(pages))) => {
                thumb(&pages[scene.frame().min(pages.len() - 1)])
            }
            (_, None, Some(Content::Video(_))) => text("▶ video").size(16).color(MUTED).into(),
            (_, None, None) => text(self.loading_note(&scene.resource))
                .size(14)
                .color(MUTED)
                .into(),
        };
        container(picture)
            .center_x(Fill)
            .center_y(Fill)
            .padding(6)
            .clip(true)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::BLACK)),
                border: Border {
                    radius: 6.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into()
    }

    fn loading_note(&self, resource: &Resource) -> String {
        match YoutubeVideo::of(resource).and_then(|v| self.downloads.0.state(&v.id)) {
            Some(download) => download.to_string(),
            None => "loading…".into(),
        }
    }

    /// Title, kind and where in it the scene is: "frame 2 of 3".
    fn describe<'a>(&self, scene: &'a Scene) -> Element<'a, Message> {
        let resource = &scene.resource;
        let mut facts = vec![resource.kind.to_string()];
        match &scene.media {
            Media::Frames { frame, count } => facts.extend(match count {
                Some(count) if count.get() == 1 => None,
                Some(count) => Some(format!("frame {} of {count}", frame + 1)),
                None => Some(format!("frame {}", frame + 1)),
            }),
            Media::Timed(timed) => {
                if let Some(duration) = timed.duration {
                    facts.push(clock(duration));
                }
            }
            Media::Failed { .. } => facts.push("failed".into()),
        }
        let mut info = column![text(&resource.title).size(20).font(BOLD).color(TEXT)].spacing(2);
        if let Some(subtitle) = &resource.subtitle {
            info = info.push(text(subtitle).size(13).color(MUTED));
        }
        info = info.push(text(facts.join(" · ")).size(14).color(MUTED));
        if let Some(download) =
            YoutubeVideo::of(resource).and_then(|v| self.downloads.0.state(&v.id))
        {
            info = info.push(download_tag(&download));
        }
        info.into()
    }

    fn transport(&self, timed: &Timed) -> Element<'_, Message> {
        let (icon, state) = match timed.playback {
            Playback::Playing => ("❚❚", tag("PLAYING", GREEN)),
            Playback::Paused => ("▶", tag("PAUSED", MUTED)),
            Playback::Ended => ("↻", tag("ENDED", AMBER)),
        };
        let play = button(text(icon).size(16).font(BOLD).center())
            .width(44)
            .padding([6, 0])
            .on_press(Message::Op(Op::PlayPause))
            .style(|_, status| filled(RAISED, TEXT, status));
        let position = self.scrub.unwrap_or(timed.position.as_secs_f64());
        let timeline: Element<'_, Message> = match timed.duration {
            Some(duration) if !duration.is_zero() => {
                slider(0.0..=duration.as_secs_f64(), position, Message::Scrub)
                    .step(0.1)
                    .on_release(Message::Scrubbed)
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
        row![play, timeline, text(times).size(14).color(TEXT), state]
            .spacing(10)
            .align_y(iced::Alignment::Center)
            .into()
    }

    fn plan_panel(&self) -> container::Container<'_, Message> {
        let engine = &self.engine.0;
        let plan = engine.plan();
        let current = engine.current_step().map(|step| step.id);
        let mut steps = column![].spacing(2);
        for (index, step) in plan.steps().iter().enumerate() {
            steps = steps.push(self.step_row(index, step, Some(step.id) == current));
        }
        if plan.is_empty() {
            steps = steps.push(
                text("The plan is empty. Preview something and press A to add it.")
                    .size(14)
                    .color(MUTED),
            );
        }
        let header = row![
            heading("PLAN", MUTED),
            text(plan.title).size(14).color(TEXT),
            Space::new().width(Fill),
            action("◀ Back", Message::Op(Op::Prev)),
            action("Next ▶", Message::Op(Op::Next)),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);
        panel(
            column![header, scrollable(steps).height(Fill)].spacing(8),
            LINE,
        )
    }

    fn step_row(&self, index: usize, step: &Step, current: bool) -> Element<'_, Message> {
        let detail = match &step.action {
            Action::Show { resource } => {
                format!("{}{}", resource.kind, self.download_note(resource))
            }
            Action::Live { .. } => "live slot".into(),
            Action::Blank => "blank".into(),
            Action::Note => "note".into(),
        };
        let marker = if current { "▶" } else { "" };
        let content = row![
            text(marker).size(14).color(RED).width(16),
            text(format!("{}", index + 1))
                .size(14)
                .color(MUTED)
                .width(24),
            text(step.label.clone())
                .size(16)
                .font(if current { BOLD } else { Font::DEFAULT })
                .color(TEXT)
                .width(Fill),
            text(detail).size(12).color(MUTED),
            button(text("✕").size(12).color(MUTED))
                .padding([2, 8])
                .on_press(Message::Remove(step.id))
                .style(|_, status| filled(Color::TRANSPARENT, MUTED, status)),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);
        button(content)
            .width(Fill)
            .padding([6, 8])
            .on_press(Message::GoTo(step.id))
            .style(move |_, status| {
                let hover = matches!(status, button::Status::Hovered);
                button::Style {
                    background: Some(Background::Color(match (current, hover) {
                        (true, _) => RED.scale_alpha(0.25),
                        (false, true) => RAISED,
                        (false, false) => Color::TRANSPARENT,
                    })),
                    text_color: TEXT,
                    border: Border {
                        color: if current { RED } else { Color::TRANSPARENT },
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..button::Style::default()
                }
            })
            .into()
    }

    /// A YouTube video's download state, e.g. " · downloading 42%", or
    /// nothing for anything else.
    fn download_note(&self, resource: &Resource) -> String {
        YoutubeVideo::of(resource)
            .and_then(|video| self.downloads.0.state(&video.id))
            .map_or_else(String::new, |download| format!(" · {download}"))
    }
}

fn download_tag(download: &Download) -> Element<'static, Message> {
    let color = match download {
        Download::Queued | Download::Running(_) => AMBER,
        Download::Ready(_) => GREEN,
        Download::Failed(_) => RED,
    };
    text(format!("YouTube: {download}"))
        .size(13)
        .color(color)
        .into()
}

fn clock(d: Duration) -> String {
    format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60)
}

fn panel<'a>(
    content: impl Into<Element<'a, Message>>,
    edge: Color,
) -> container::Container<'a, Message> {
    container(content)
        .padding(12)
        .height(Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(PANEL)),
            border: Border {
                color: edge,
                width: 1.5,
                radius: 10.0.into(),
            },
            ..container::Style::default()
        })
}

fn heading(label: &str, color: Color) -> Element<'_, Message> {
    text(label).size(13).font(BOLD).color(color).into()
}

fn tag(label: &str, color: Color) -> Element<'_, Message> {
    container(text(label).size(12).font(BOLD).color(TEXT))
        .padding([2, 8])
        .style(move |_| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

fn empty_monitor(note: &str) -> Element<'_, Message> {
    container(text(note).size(14).color(FAINT))
        .center_x(Fill)
        .center_y(Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(Color::BLACK.scale_alpha(0.4))),
            border: Border {
                color: LINE,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn thumb(handle: &image::Handle) -> Element<'static, Message> {
    image(handle.clone())
        .width(Fill)
        .height(Fill)
        .content_fit(iced::ContentFit::Contain)
        .into()
}

fn action(label: &str, message: Message) -> button::Button<'_, Message> {
    action_tinted(label, RAISED, Some(message))
}

fn action_tinted(
    label: &str,
    color: Color,
    message: Option<Message>,
) -> button::Button<'_, Message> {
    button(text(label).size(15).font(BOLD).center())
        .padding([10, 14])
        .on_press_maybe(message)
        .style(move |_, status| filled(color, TEXT, status))
}

fn wide(button: button::Button<'_, Message>) -> button::Button<'_, Message> {
    button.width(Fill)
}

fn filled(color: Color, text_color: Color, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => lighten(color),
        button::Status::Disabled => color.scale_alpha(0.3),
        _ => color,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: if status == button::Status::Disabled {
            text_color.scale_alpha(0.4)
        } else {
            text_color
        },
        border: Border {
            radius: 8.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

fn lighten(color: Color) -> Color {
    Color {
        r: (color.r + 0.06).min(1.0),
        g: (color.g + 0.06).min(1.0),
        b: (color.b + 0.06).min(1.0),
        a: color.a.max(0.3),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressed(key: Key, status: event::Status) -> Option<Message> {
        let event = iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            text: None,
            repeat: false,
        });
        key_event(event, status, iced::window::Id::unique())
    }

    fn op(message: Option<Message>) -> Option<Op> {
        match message {
            Some(Message::Op(op)) => Some(op),
            _ => None,
        }
    }

    #[test]
    fn keys_map_to_ops_unless_the_search_box_used_them() {
        let free = event::Status::Ignored;
        assert_eq!(
            op(pressed(Key::Character("b".into()), free)),
            Some(Op::Blackout)
        );
        assert_eq!(
            op(pressed(Key::Character("B".into()), free)),
            Some(Op::Blackout)
        );
        assert_eq!(
            op(pressed(Key::Named(Named::Space), free)),
            Some(Op::PlayPause)
        );
        assert_eq!(
            op(pressed(Key::Named(Named::PageDown), free)),
            Some(Op::Next)
        );
        assert_eq!(
            op(pressed(Key::Named(Named::ArrowLeft), free)),
            Some(Op::Prev)
        );
        assert_eq!(op(pressed(Key::Character("q".into()), free)), None);
        let typing = event::Status::Captured;
        assert_eq!(op(pressed(Key::Character("b".into()), typing)), None);
    }

    #[test]
    fn script_key_names_match_the_table() {
        assert_eq!(op_named("Enter"), Some(Op::Enter));
        assert_eq!(op_named("pagedown"), Some(Op::Next));
        assert_eq!(op_named("b"), Some(Op::Blackout));
        assert_eq!(op_named("f13"), None);
    }
}
