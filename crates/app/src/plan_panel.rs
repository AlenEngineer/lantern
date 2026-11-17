//! The open plan's own settings, beside the plan: its name, when it
//! starts, how long it should take, whether it is a template; the media
//! only it needs; its look where that differs from the look for all
//! plans; a copy of it, and deleting it.
//!
//! They sit where the side panel is, opened from the plan's name. What
//! holds for this computer and for every plan is in Settings.

use std::path::Path;

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Fill};

use crate::look::{self, BODY, BOLD, Icon, META, Tone};
use crate::side::{self, Field, From};
use crate::widgets;
use crate::{App, Message, length, library};

/// A change made in the plan's panel.
#[derive(Debug, Clone)]
pub enum PlanEdit {
    /// Opens the panel, or closes it.
    Toggle,
    Close,
    /// How long the plan should take, as typed.
    Target(String),
    /// The path of its own folder N, as typed.
    Folder(usize, String),
    /// What is typed is meant: Enter was pressed.
    Done,
    AddFolder,
    RemoveFolder(usize),
    /// It searches the Bible of this name, or stops.
    Bible(String),
}

/// What is being typed in the panel and isn't in the plan yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Typing {
    Target,
    Folder(usize),
}

/// How wide the names of the fields are.
const NAMES: f32 = 92.0;

/// The field of folder `at`, to put the cursor in.
fn folder_box(at: usize) -> String {
    format!("plan-folder-{at}")
}

impl App {
    pub(crate) fn plan_edit(&mut self, edit: PlanEdit) -> iced::Task<Message> {
        match edit {
            PlanEdit::Toggle if self.plan_panel => return self.plan_edit(PlanEdit::Close),
            PlanEdit::Toggle => {
                self.plan_panel = true;
                // The panel has the side to itself.
                self.filling = None;
                self.look_typed = None;
                self.starts_typed = None;
                self.plan_typed = None;
                self.deleting_plan = false;
            }
            PlanEdit::Close => {
                self.keep_typed();
                self.plan_panel = false;
                self.look_typed = None;
                self.starts_typed = None;
                self.deleting_plan = false;
            }
            PlanEdit::Target(typed) => {
                // Kept as typed until it reads as a length. Emptied, the
                // plan has no target.
                if length::typeable(&typed) {
                    if let Some(target) = length::parse(&typed) {
                        self.engine.0.edit_plan(|plan| plan.target = target);
                    }
                    self.plan_typed = Some((Typing::Target, typed));
                }
            }
            PlanEdit::Folder(at, typed) => {
                // Another row was being typed in: that one is meant.
                if self.plan_typed.as_ref().map(|(typing, _)| *typing) != Some(Typing::Folder(at)) {
                    self.keep_typed();
                }
                self.plan_typed = Some((Typing::Folder(at), typed));
            }
            PlanEdit::Done => self.keep_typed(),
            PlanEdit::AddFolder => {
                self.keep_typed();
                let at = self.engine.0.plan().folders.len();
                self.plan_typed = Some((Typing::Folder(at), String::new()));
                return iced::widget::operation::focus(folder_box(at));
            }
            PlanEdit::RemoveFolder(at) => {
                // A row not in the plan yet is only taken back.
                let typing = self.plan_typed.take();
                let folders = self.engine.0.plan().folders.len();
                if at < folders {
                    self.remember(None);
                    self.engine.0.edit_plan(|plan| {
                        plan.folders.remove(at);
                    });
                    self.sync_library();
                } else if typing.as_ref().map(|(typing, _)| *typing) != Some(Typing::Folder(at)) {
                    self.plan_typed = typing;
                }
            }
            PlanEdit::Bible(name) => {
                self.remember(None);
                self.engine.0.edit_plan(|plan| {
                    match plan
                        .bibles
                        .iter()
                        .position(|bible| bible.eq_ignore_ascii_case(&name))
                    {
                        Some(at) => {
                            plan.bibles.remove(at);
                        }
                        None => plan.bibles.push(name),
                    }
                });
                self.sync_library();
            }
        }
        iced::Task::none()
    }

    /// Puts the folder being typed into the plan, as typed (`~/…`, so
    /// the plan reads the same on another computer). Typed empty, the
    /// folder is gone. The library then looks in what the plan has now.
    fn keep_typed(&mut self) {
        let Some((Typing::Folder(at), typed)) = self.plan_typed.take() else {
            return;
        };
        let typed = typed.trim().to_string();
        let folders = self.engine.0.plan().folders;
        let now = match (folders.get(at), typed.is_empty()) {
            (Some(_), true) => Some(None),
            (Some(was), false) if was.as_path() != Path::new(&typed) => Some(Some(typed)),
            (None, false) => Some(Some(typed)),
            _ => None,
        };
        let Some(now) = now else {
            return;
        };
        self.remember(None);
        self.engine.0.edit_plan(|plan| match now {
            Some(path) if at < plan.folders.len() => plan.folders[at] = path.into(),
            Some(path) => plan.folders.push(path.into()),
            None => {
                plan.folders.remove(at);
            }
        });
        self.sync_library();
    }

    /// The panel, while it is open.
    pub(crate) fn plan_card(&self) -> Option<Element<'_, Message>> {
        if !self.plan_panel {
            return None;
        }
        let p = self.palette;
        let plan = self.engine.0.plan();
        let edit = |edit: PlanEdit| Message::PlanPanel(edit);
        let name = |words: &'static str| {
            text(words)
                .size(META)
                .color(p.on_surface_variant)
                .width(NAMES)
        };
        let heading = |words: &'static str| widgets::caption(p, words);
        let rule = || widgets::rule(p);
        let input = |hint: &'static str, value: &str| {
            text_input(hint, value)
                .size(META)
                .padding([5, 9])
                .style(look::input(p))
        };

        let head = row![
            look::icon(Icon::Tune, 16.0, p.primary),
            text("This plan").size(BODY).font(BOLD).color(p.primary),
            Space::new().width(Fill),
            button(text("✕").size(BODY).color(p.on_surface_variant))
                .padding([2, 8])
                .on_press(edit(PlanEdit::Close))
                .style(look::row(p, Tone::Plain)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        // As typed while it doesn't read as a time yet.
        let starts = match (&self.starts_typed, plan.starts) {
            (Some(typed), _) => typed.clone(),
            (None, Some(starts)) => starts.to_string(),
            (None, None) => String::new(),
        };
        let target = match (&self.plan_typed, plan.target) {
            (Some((Typing::Target, typed)), _) => typed.clone(),
            (_, Some(target)) => length::clock(target),
            (_, None) => String::new(),
        };
        let template = button(widgets::switch(p, plan.template))
            .padding(0)
            .on_press(Message::PlanTemplate(!plan.template))
            .style(look::row(p, Tone::Plain));
        let about = column![
            line(
                name("Name").into(),
                input("Plan name", &plan.title)
                    .on_input(Message::RenamePlan)
                    .into()
            ),
            line(
                name("Starts").into(),
                input("10:30", &starts)
                    .on_input(Message::PlanStarts)
                    .width(72)
                    .into()
            ),
            line(
                name("Target").into(),
                input("45:00", &target)
                    .on_input(move |typed| edit(PlanEdit::Target(typed)))
                    .width(72)
                    .into()
            ),
            line(name("Template").into(), template.into()),
        ]
        .spacing(8);

        // Its own folders, each as typed; the one being typed in may not
        // be in the plan yet.
        let mut media = column![heading("ITS MEDIA")].spacing(6);
        let mut rows: Vec<String> = plan
            .folders
            .iter()
            .map(|folder| folder.display().to_string())
            .collect();
        if let Some((Typing::Folder(at), typed)) = &self.plan_typed {
            match rows.get_mut(*at) {
                Some(row) => row.clone_from(typed),
                None => rows.push(typed.clone()),
            }
        }
        let config_dir = self
            .config_status
            .path()
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        for (at, folder) in rows.iter().enumerate() {
            // One that isn't on this computer says so by its colour.
            let typing =
                self.plan_typed.as_ref().map(|(typing, _)| *typing) == Some(Typing::Folder(at));
            let there = folder.trim().is_empty()
                || crate::config::resolve(
                    Path::new(folder.trim()),
                    &config_dir,
                    self.home.as_deref(),
                )
                .is_dir();
            let ink = if there || typing {
                p.on_surface_variant
            } else {
                p.error
            };
            media = media.push(
                row![
                    look::icon(Icon::Folder, 14.0, ink),
                    input("~/Church/Campaña", folder)
                        .id(folder_box(at))
                        .on_input(move |typed| edit(PlanEdit::Folder(at, typed)))
                        .on_submit(edit(PlanEdit::Done))
                        .width(Fill),
                    button(text("✕").size(META).color(p.on_surface_variant))
                        .padding([2, 6])
                        .on_press(edit(PlanEdit::RemoveFolder(at)))
                        .style(look::row(p, Tone::Plain)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
        media = media.push(
            button(
                row![
                    text("+").size(META).color(p.on_surface_variant),
                    look::icon(Icon::Folder, 14.0, p.on_surface_variant),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding([3, 9])
            .on_press(edit(PlanEdit::AddFolder))
            .style(look::row(p, Tone::Plain)),
        );
        // The Bibles there are: the ones it searches are lit. None lit,
        // it searches them all.
        let bibles: Vec<String> = self
            .config
            .bibles
            .iter()
            .map(|bible| library::name_of_bible(bible.name.as_deref(), &bible.path))
            .collect();
        if !bibles.is_empty() {
            let mut picks = row![look::icon(Icon::Book, 14.0, p.on_surface_variant)]
                .spacing(6)
                .align_y(Alignment::Center);
            for bible in bibles {
                let on = plan
                    .bibles
                    .iter()
                    .any(|picked| picked.eq_ignore_ascii_case(&bible));
                picks = picks.push(
                    button(text(bible.clone()).size(META).font(if on {
                        BOLD
                    } else {
                        look::MEDIUM
                    }))
                    .padding([3, 9])
                    .on_press(edit(PlanEdit::Bible(bible)))
                    .style(look::pill(p, if on { Tone::Selected } else { Tone::Plain })),
                );
            }
            media = media.push(picks.wrap());
        }

        // Its look: each value comes from the look for all plans, or is
        // set here.
        let usual = &self.config.stage;
        let look = plan.look.clone().unwrap_or_else(|| usual.clone());
        let mut its_look = column![heading("ITS LOOK")].spacing(6);
        for field in Field::ALL {
            let here = matches!(side::origin(&plan, usual, None, field), From::Here);
            let typed = self.look_shown(None, field, &look);
            let control = side::look_control(p, field, &look, &typed);
            let from: Element<'_, Message> = if here {
                side::set_here(p, field, 11.0, "Back to the look for all plans")
            } else {
                text("default")
                    .size(11.0)
                    .color(p.on_surface_variant)
                    .into()
            };
            its_look = its_look.push(
                row![
                    text(field.name())
                        .size(META)
                        .font(if here { BOLD } else { look::MEDIUM })
                        .color(if here {
                            p.primary
                        } else {
                            p.on_surface_variant
                        })
                        .width(NAMES),
                    container(control).width(Fill),
                    container(from)
                        .width(62)
                        .align_x(iced::alignment::Horizontal::Right),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }

        let act = |words: &'static str, tone: Tone, message: Option<Message>| {
            button(container(text(words).size(META).font(look::MEDIUM)).center_x(Fill))
                .width(Fill)
                .padding([8, 0])
                .on_press_maybe(message)
                .style(look::pill(p, tone))
        };
        let foot = row![
            act("Make a copy", Tone::Plain, Some(Message::DuplicatePlan)),
            act(
                if self.copying {
                    "Exporting…"
                } else {
                    "Export"
                },
                Tone::Plain,
                (!self.copying).then_some(Message::ExportPlan)
            ),
            act(
                if self.deleting_plan {
                    "For good?"
                } else {
                    "Delete"
                },
                Tone::Danger,
                Some(Message::DeletePlan)
            ),
        ]
        .spacing(8);

        Some(
            column![
                head,
                // Room at the right for the bar that scrolls it.
                scrollable(
                    container(column![about, rule(), media, rule(), its_look].spacing(12))
                        .padding(iced::Padding::default().right(10.0))
                )
                .height(Fill),
                foot,
            ]
            .spacing(12)
            .height(Fill)
            .into(),
        )
    }
}

/// A field's name and the field, on one line.
fn line<'a>(
    name: Element<'a, Message>,
    control: Element<'a, Message>,
) -> iced::widget::Row<'a, Message> {
    row![name, control].spacing(10).align_y(Alignment::Center)
}
