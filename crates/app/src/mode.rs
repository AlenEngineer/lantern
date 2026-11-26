//! The three things Lantern is used for, kept apart.
//!
//! - **Plan**: making and managing plans. Nothing reaches the projector.
//! - **Run**: a plan in front of people. The plan is locked, unless it is
//!   unlocked for a change to what is still ahead.
//! - **Live**: no plan. What is picked is shown, and what was shown can be
//!   saved as a plan afterwards.
//!
//! The limits are enforced where messages and key operations come in
//! ([`App::refused`], [`App::op_in_mode`]), not by hiding buttons: a key, a
//! script and the phone meet the same wall.

use std::time::{Duration, Instant};

use iced::widget::{Space, button, column, container, row, scrollable, text, tooltip};
use iced::{Alignment, Element, Fill, Task};
use lantern_engine::{FlowOf, Plan, StepId};

use crate::keys::{self, Op};
use crate::look::{self, BODY, BOLD, Icon, MEDIUM, META, PILL, Tone};
use crate::plans::{self, Notice};
use crate::widgets::{chip, hinted, pill, single_line};
use crate::{App, Message, log};

/// What Lantern is being used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Plan,
    Run,
    Live,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Plan, Mode::Run, Mode::Live];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Plan => "Plan",
            Mode::Run => "Run",
            Mode::Live => "Live",
        }
    }

    /// The mode a script or a config names.
    pub fn parse(name: &str) -> Option<Mode> {
        Mode::ALL
            .into_iter()
            .find(|mode| mode.name().eq_ignore_ascii_case(name.trim()))
    }

    /// Its colour on the switch: the plan's own, the stage's, and one for
    /// showing with no plan.
    fn tone(self) -> Tone {
        match self {
            Mode::Plan => Tone::Solid,
            Mode::Run => Tone::Live,
            Mode::Live => Tone::Alarm,
        }
    }
}

/// How long "End the service" waits for its second press.
const SURE: Duration = Duration::from_secs(4);

/// How wide each pill of the switch is, and the room between two. Every
/// pill is as wide as the others, so the lit one under them can slide
/// from one to the next.
const PILL_WIDE: f32 = 62.0;
const PILL_GAP: f32 = 2.0;
/// How wide the switch is with every mode on it.
pub(crate) const SWITCH: f32 = Mode::ALL.len() as f32 * (PILL_WIDE + PILL_GAP) - PILL_GAP;

/// What kind of thing a message does, as far as the modes care.
enum Does {
    /// Puts something on the projector, or moves what is there.
    Shows,
    /// Changes a step of the plan.
    EditsStep(StepId),
    /// Changes the plan as a whole, or adds to it.
    EditsPlan,
    /// Opens, makes, deletes or leaves a plan.
    ManagesPlans,
    /// Neither: looking, searching, settings.
    Neither,
}

fn does(message: &Message) -> Does {
    use Message as M;
    match message {
        M::Resume
        | M::GotoFrame(_)
        | M::Attachment(_)
        | M::BackToSlides { .. }
        | M::Scrub(_)
        | M::Scrubbed => Does::Shows,
        M::Remove(id)
        | M::Rename(id, _)
        | M::SetJoined(FlowOf::Step(id), _)
        | M::SetRepeat(FlowOf::Step(id), _)
        | M::SetShuffle(FlowOf::Step(id), _)
        | M::SectionColor(id, _)
        | M::Rail(
            crate::rail::Act::Unloop(FlowOf::Step(id))
            | crate::rail::Act::Unshuffle(FlowOf::Step(id))
            | crate::rail::Act::Ends(FlowOf::Step(id), _),
        )
        | M::StepSound(id, _)
        | M::SetFade(id, ..)
        | M::SoundFade(id, _)
        | M::KeepCopy(id)
        | M::SoundFrom(id, _)
        | M::SlotSearch(id, _)
        | M::Length(id, _)
        | M::Move(id, _)
        | M::ReplaceStep(id)
        | M::EditStep(Some((id, _))) => Does::EditsStep(*id),
        M::AddStep(_)
        | M::Unstandby(_)
        | M::EditStarts
        | M::PlanStarts(_)
        | M::PlanTarget(_)
        | M::PlanGoal(_)
        | M::Fill(crate::fill_panel::FillEdit::Accept)
        | M::Rail(
            crate::rail::Act::Unloop(FlowOf::Plan | FlowOf::Span(_))
            | crate::rail::Act::Unshuffle(FlowOf::Plan | FlowOf::Span(_))
            | crate::rail::Act::Ends(FlowOf::Plan | FlowOf::Span(_), _),
        )
        | M::PlanPanel(
            crate::plan_panel::PlanEdit::Folder(..)
            | crate::plan_panel::PlanEdit::Done
            | crate::plan_panel::PlanEdit::AddFolder
            | crate::plan_panel::PlanEdit::RemoveFolder(_)
            | crate::plan_panel::PlanEdit::Bible(_),
        )
        | M::LookEdit(..)
        | M::LookReset(_)
        | M::EditTarget
        | M::SetRepeat(FlowOf::Plan | FlowOf::Span(_), _)
        | M::SetShuffle(FlowOf::Plan | FlowOf::Span(_), _)
        | M::SetJoined(FlowOf::Plan | FlowOf::Span(_), _)
        | M::NoteFor(_)
        | M::NoteEdit(_)
        | M::NoteGone(_) => Does::EditsPlan,
        M::PlanTemplate(_)
        | M::DuplicatePlan
        | M::DeletePlan
        | M::ToggleImport
        | M::Import(_)
        | M::FromTemplate(_)
        | M::SetTemplate(..)
        | M::OpenPlan(_)
        | M::DiscardPlan
        | M::LeavePlan
        | M::ClosePlanMode
        | M::EditPlanName
        | M::PlanName(_)
        | M::SubmitPlanName => Does::ManagesPlans,
        _ => Does::Neither,
    }
}

/// A refusal's reason as a sentence starts: "The plan is locked…".
fn capitalised(why: &str) -> String {
    let mut letters = why.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

impl App {
    /// Whether the plan can be changed now: always while planning, and in
    /// a run only once it is unlocked.
    pub(crate) fn can_edit(&self) -> bool {
        match self.mode {
            Mode::Plan => true,
            Mode::Run => self.unlocked.is_some(),
            Mode::Live => false,
        }
    }

    /// Whether a step is still ahead of the service: the only steps a run
    /// lets be changed. Before the service starts, all of them are.
    pub(crate) fn ahead(&self, plan: &Plan, id: StepId) -> bool {
        let current = self
            .engine
            .0
            .current_step()
            .and_then(|at| plan.position(at.id));
        match (current, plan.position(id)) {
            (Some(current), Some(step)) => step > current,
            (None, Some(_)) => true,
            (_, None) => false,
        }
    }

    /// Why `message` can't be done in this mode; `None` when it can.
    pub(crate) fn refused(&self, message: &Message) -> Option<&'static str> {
        // The phone runs a service or shows on the fly. It doesn't plan.
        if let Message::Remote(command) = message {
            let harmless = matches!(
                command,
                lantern_remote::Command::Hello | lantern_remote::Command::Search { .. }
            );
            return (self.mode == Mode::Plan && !harmless)
                .then_some("the phone waits until the service starts");
        }
        match (self.mode, does(message)) {
            (_, Does::Neither) => None,
            (Mode::Plan, Does::Shows) => Some("nothing reaches the projector while planning"),
            (Mode::Plan, _) => None,
            (Mode::Run | Mode::Live, Does::Shows) => None,
            (Mode::Run, Does::ManagesPlans) => Some("end the service first"),
            (Mode::Run, Does::EditsPlan) if self.unlocked.is_some() => None,
            (Mode::Run, Does::EditsStep(id)) if self.unlocked.is_some() => (!self
                .ahead(&self.engine.0.plan(), id))
            .then_some("only what is still ahead can change"),
            (Mode::Run, _) => Some("the plan is locked while it runs"),
            (Mode::Live, _) => Some("there is no plan in Live"),
        }
    }

    /// What `op` becomes in this mode, or why it can't be done.
    ///
    /// While planning, taking a search result adds it to the plan: that is
    /// what picking something is for there.
    pub(crate) fn op_in_mode(&self, op: Op) -> Result<Op, &'static str> {
        let shows = matches!(
            op,
            Op::Take | Op::Next | Op::Prev | Op::PlayPause | Op::Clear
        );
        let edits = matches!(
            op,
            Op::AddToPlan
                | Op::Standby
                | Op::MoveUp
                | Op::MoveDown
                | Op::NestIn
                | Op::NestOut
                | Op::RemoveStep
                | Op::Rename
                | Op::Duplicate
                | Op::Undo
                | Op::Redo
        );
        let manages = matches!(op, Op::New | Op::Open);
        // → and ← are a tree's keys only where nothing they could move
        // is on the projector.
        let op = match (self.mode, op) {
            (Mode::Run | Mode::Live, Op::In) => Op::Next,
            (Mode::Run | Mode::Live, Op::Out) => Op::Prev,
            (Mode::Run | Mode::Live, Op::Pane) => {
                return Err("the library is beside the plan while planning");
            }
            (_, op) => op,
        };
        // The switch, from a key: the same ways in and out as its buttons.
        if let Op::Mode(to) = op {
            let runs = self
                .engine
                .0
                .plan()
                .steps()
                .iter()
                .any(|step| !step.is_section());
            return match (self.mode, to) {
                (Mode::Run, Mode::Live) => Err("end the service first"),
                (Mode::Live, Mode::Run) => Err("a service starts from Plan"),
                (Mode::Plan, Mode::Run) if !runs => Err("the plan has nothing to run yet"),
                _ => Ok(op),
            };
        }
        match self.mode {
            Mode::Plan
                if op == Op::Take
                    && (self.search.picked().is_some() || self.shelf_pick.is_some()) =>
            {
                Ok(Op::AddToPlan)
            }
            Mode::Plan if shows => Err("nothing reaches the projector while planning"),
            Mode::Plan if op == Op::Unlock => Err("nothing is locked while planning"),
            Mode::Plan => Ok(op),
            Mode::Run if manages => Err("end the service first"),
            Mode::Run if edits && self.unlocked.is_none() => {
                Err("the plan is locked while it runs")
            }
            Mode::Run
                if matches!(
                    op,
                    Op::MoveUp
                        | Op::MoveDown
                        | Op::NestIn
                        | Op::NestOut
                        | Op::RemoveStep
                        | Op::Rename
                ) =>
            {
                let plan = self.engine.0.plan();
                match self.selected {
                    Some(step) if self.ahead(&plan, step) => Ok(op),
                    _ => Err("only what is still ahead can change"),
                }
            }
            Mode::Run => Ok(op),
            Mode::Live if edits || manages || matches!(op, Op::Save | Op::Unlock) => {
                Err("there is no plan in Live")
            }
            Mode::Live => Ok(op),
        }
    }

    /// Locks the plan again once the service has moved on from the step
    /// it was unlocked at: an unlock is for one change, not for the rest
    /// of the service.
    pub(crate) fn sync_lock(&mut self) {
        let now = self.engine.0.current_step().map(|step| step.id);
        if self.unlocked.is_some_and(|at| at != now) || self.mode != Mode::Run {
            self.unlocked = None;
        }
        if self.ending.is_some_and(|asked| asked.elapsed() > SURE) {
            self.ending = None;
        }
    }

    /// The Plan or Live button of the switch. Run is entered by starting
    /// the service and left by ending it.
    pub(crate) fn set_mode(&mut self, to: Mode) -> Task<Message> {
        self.starting = None;
        self.rehearsal = None;
        self.rehearsed = None;
        self.filling = None;
        self.plan_panel = false;
        self.ring = None;
        self.pull = None;
        self.plan_typed = None;
        match (self.mode, to) {
            (from, to) if from == to => {}
            (Mode::Run, _) | (_, Mode::Run) => {}
            (_, to) => {
                // Whatever was on the projector goes with the mode that
                // put it there.
                self.engine.0.clear();
                self.engine.0.clear_preview();
                self.engine.0.rewind();
                self.selected = None;
                self.search.unpick();
                self.live_from = self.engine.0.log().len();
                self.mode = to;
            }
        }
        self.preload()
    }

    /// From planning to running: the plan's first step goes on.
    pub(crate) fn start_service(&mut self) -> Task<Message> {
        let plan = self.engine.0.plan();
        if self.mode != Mode::Plan || !plan.steps().iter().any(|step| !step.is_section()) {
            return Task::none();
        }
        // The pane shows the plan's steps, whatever it was showing.
        if !matches!(self.plans.mode, plans::Mode::Steps) {
            self.plans.cancel();
            self.plans.mode = plans::Mode::Steps;
        }
        // The look round, once: what it finds is said, and the next ask
        // starts the service anyway.
        let sound = plan
            .steps()
            .iter()
            .any(|step| match &step.action {
                lantern_engine::Action::Show { resource } => {
                    step.sound.is_some() || crate::stage::has_sound(resource)
                }
                _ => false,
            })
            .then(crate::before::sound)
            .flatten();
        if self.starting.take().is_none() {
            let issues: Vec<crate::before::Trouble> = self
                .issues(&plan)
                .into_iter()
                .map(|issue| crate::before::Trouble {
                    level: issue.level,
                    label: plan
                        .get(issue.step)
                        .map_or_else(String::new, |step| step.label.clone()),
                    what: issue.what,
                })
                .collect();
            let last = self.last_sound.of(&self.setup);
            let worries = crate::before::worries(&issues, sound.as_ref(), last.as_deref());
            if !worries.is_empty() {
                log(format!("mode: before the service starts: {worries:?}"));
                self.starting = Some(worries);
                return Task::none();
            }
        }
        if let Some(sound) = sound.filter(|sound| !sound.output.is_empty())
            && let Err(error) = self.last_sound.keep(&self.setup, &sound.output)
        {
            log(format!("mode: the sound output was not kept: {error}"));
        }
        self.rehearsal = None;
        self.rehearsed = None;
        self.filling = None;
        self.plan_panel = false;
        self.ring = None;
        self.pull = None;
        self.plan_typed = None;
        self.editing = None;
        self.selected = None;
        self.search.unpick();
        self.search_open = false;
        self.engine.0.clear_preview();
        self.engine.0.rewind();
        self.mode = Mode::Run;
        self.opened.clear();
        self.unlocked = None;
        self.ending = None;
        log("mode: the service starts");
        self.op(Op::Next)
    }

    /// From running back to planning. Asked twice: the first press only
    /// arms it, so one stray click doesn't empty the projector.
    pub(crate) fn end_service(&mut self) -> Task<Message> {
        if self.mode != Mode::Run {
            return Task::none();
        }
        if self.ending.take().is_none() {
            self.ending = Some(Instant::now());
            return Task::none();
        }
        self.engine.0.clear();
        self.engine.0.clear_preview();
        self.engine.0.rewind();
        self.selected = None;
        self.unlocked = None;
        self.resuming = None;
        self.mode = Mode::Plan;
        log("mode: the service ended");
        self.preload()
    }

    /// Unlocks the running plan for a change to what is ahead, or locks
    /// it again.
    pub(crate) fn toggle_unlock(&mut self) {
        if self.mode != Mode::Run {
            return;
        }
        self.unlocked = match self.unlocked {
            Some(_) => {
                self.editing = None;
                None
            }
            None => Some(self.engine.0.current_step().map(|step| step.id)),
        };
    }

    /// Makes a plan of what was shown this session, in the order it was
    /// shown, and opens it for planning.
    pub(crate) fn save_session(&mut self) -> Task<Message> {
        if self.mode != Mode::Live {
            return Task::none();
        }
        let mut plan = Plan::new(format!("Sesión {}", plans::today()));
        let mut last = None;
        for entry in self.engine.0.log().into_iter().skip(self.live_from) {
            // Shown twice running is one step.
            if last.as_ref() == Some(&entry.resource.id) {
                continue;
            }
            last = Some(entry.resource.id.clone());
            plan.show(entry.resource);
        }
        if plan.is_empty() {
            return Task::none();
        }
        match self.plans.store().adopt(plan) {
            Ok(path) => {
                let _ = self.set_mode(Mode::Plan);
                let replaced = self.plans.switch(&self.engine.0, plans::Switch::Open(path));
                self.switched(replaced)
            }
            Err(err) => {
                self.plans.notice = Some(Notice::Failed(format!("Couldn't save it: {err}")));
                Task::none()
            }
        }
    }

    /// The switch between the modes, and in a run the way out of it.
    pub(crate) fn mode_switch(&self) -> Element<'_, Message> {
        let p = self.palette;
        // In a run only Run shows: the others are reached by ending it,
        // and the strip has the service's time to show instead.
        let shown: Vec<Mode> = Mode::ALL
            .into_iter()
            .filter(|mode| self.mode != Mode::Run || *mode == Mode::Run)
            .collect();
        const TALL: f32 = 32.0;
        let place = |mode: Mode| {
            let at = shown.iter().position(|shown| *shown == mode).unwrap_or(0);
            at as f32 * (PILL_WIDE + PILL_GAP)
        };
        let (fill, ink) = look::pill_colours(p, self.mode.tone());
        // On its way from the mode it left: across, and from its colour.
        let (aside, fill) = match self.motion.slide() {
            Some((from, along)) => (
                crate::motion::between(place(from), place(self.mode), along),
                crate::motion::blend(look::pill_colours(p, from.tone()).0, fill, along),
            ),
            None => (place(self.mode), fill),
        };
        // Until the light arrives, the mode it is going to keeps a plain
        // pill's ground and takes its ink as the light comes: dark ink on
        // the dark strip, it was gone for the length of the slide.
        let (plain_fill, plain_ink) = look::pill_colours(p, Tone::Plain);
        let (ink, waiting) = match self.motion.slide() {
            Some((_, along)) => (
                crate::motion::blend(plain_ink, ink, along),
                Some(plain_fill),
            ),
            None => (ink, None),
        };
        let ground = row![
            Space::new().width(place(self.mode)),
            container(Space::new().width(PILL_WIDE).height(TALL)).style(move |_| {
                container::Style {
                    background: waiting.map(Into::into),
                    border: iced::Border {
                        radius: (TALL / 2.0).into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }
            }),
        ]
        // The first layer gives the stack its size: as wide as every pill.
        .width(shown.len() as f32 * (PILL_WIDE + PILL_GAP) - PILL_GAP)
        .height(TALL);
        let lit = row![
            Space::new().width(aside),
            container(Space::new().width(PILL_WIDE).height(TALL)).style(move |_| {
                container::Style {
                    background: Some(fill.into()),
                    border: iced::Border {
                        radius: (TALL / 2.0).into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }
            }),
        ]
        .width(shown.len() as f32 * (PILL_WIDE + PILL_GAP) - PILL_GAP)
        .height(TALL);
        let plan_has_steps = self
            .engine
            .0
            .plan()
            .steps()
            .iter()
            .any(|step| !step.is_section());
        let buttons = shown.iter().map(|&mode| {
            let on = mode == self.mode;
            // Run is reached by starting the service: pressed, it starts
            // it, as Start the service does. The mode it is in is
            // pressable too, to no effect: unpressable, it would look
            // switched off.
            let starts = !on && mode == Mode::Run;
            let can_start = plan_has_steps;
            let label = container(
                text(mode.name())
                    .size(BODY)
                    .font(if on { BOLD } else { MEDIUM })
                    .wrapping(text::Wrapping::None),
            )
            .center_x(Fill)
            .center_y(Fill);
            let pill = button(label)
                .width(PILL_WIDE)
                .height(TALL)
                .padding(0)
                .on_press_maybe(match starts {
                    true => can_start.then_some(Message::StartService),
                    false => Some(Message::SetMode(mode)),
                });
            let pill: Element<'_, Message> = match on {
                true => pill.style(look::pill_clear(ink)).into(),
                false => pill.style(look::pill(p, Tone::Plain)).into(),
            };
            let does = match mode {
                Mode::Plan => "Make and manage plans",
                Mode::Run => "Start the service",
                Mode::Live => "Show with no plan",
            };
            hinted(
                p,
                pill,
                keys::with_key(does, Op::Mode(mode)),
                tooltip::Position::Bottom,
            )
        });
        let mut switch = row![iced::widget::stack![
            ground,
            lit,
            row(buttons).spacing(PILL_GAP)
        ]]
        .spacing(8)
        .align_y(Alignment::Center);
        match self.mode {
            Mode::Run => {
                let sure = self.ending.is_some();
                switch = switch.push(hinted(
                    p,
                    button(
                        text(if sure {
                            "Again to end it"
                        } else {
                            "End the service"
                        })
                        .size(BODY)
                        .font(MEDIUM)
                        .wrapping(text::Wrapping::None),
                    )
                    .padding([6, 14])
                    .on_press(Message::EndService)
                    .style(look::pill(p, if sure { Tone::Alarm } else { Tone::Plain })),
                    keys::with_key("Back to planning", Op::Mode(Mode::Plan)),
                    tooltip::Position::Bottom,
                ));
            }
            Mode::Plan | Mode::Live => {}
        }
        switch.into()
    }

    /// The line at the top of the pane. It has no standing words: it says
    /// why something was just refused, and in a run it holds Unlock. With
    /// neither, it takes no room, but it is always there, so the pane's
    /// other children keep their places.
    pub(crate) fn mode_note(&self) -> Element<'_, Message> {
        let p = self.palette;
        // The look round found something: what, and the two ways on.
        if let Some(worries) = &self.starting {
            let mut card = column![
                text("Before you start")
                    .size(META)
                    .font(BOLD)
                    .color(p.tertiary)
            ]
            .spacing(6);
            for worry in worries {
                card = card.push(text(worry.clone()).size(META).color(p.on_surface));
            }
            card = card.push(
                row![
                    pill(
                        p,
                        "Start anyway",
                        Tone::Live,
                        PILL,
                        Some(Message::StartService)
                    )
                    .width(Fill),
                    pill(p, "Not yet", Tone::Plain, PILL, Some(Message::NotYet)).width(Fill),
                ]
                .spacing(6),
            );
            return container(card)
                .padding([8, 10])
                .style(move |_| container::Style {
                    background: Some(p.surface_container_highest.into()),
                    border: iced::Border {
                        radius: 12.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                })
                .into();
        }
        // A service cut short: where it was, and the way back to it.
        if let Some((step, frame)) = self.resuming {
            let label = self
                .engine
                .0
                .plan()
                .get(step)
                .map_or_else(String::new, |step| step.label.clone());
            let slide = match frame {
                0 => String::new(),
                frame => format!(", slide {}", frame + 1),
            };
            return container(
                column![
                    text(format!("The service was at “{label}”{slide}."))
                        .size(META)
                        .color(p.on_surface),
                    text("Nothing is on the projector until you pick it up.")
                        .size(META)
                        .color(p.on_surface_variant),
                    pill(
                        p,
                        &keys::with_key("Pick up there", Op::Next),
                        Tone::Live,
                        PILL,
                        Some(Message::Key {
                            op: Op::Next,
                            only_if_empty: false,
                        }),
                    )
                    .width(Fill),
                ]
                .spacing(6),
            )
            .padding([8, 10])
            .style(move |_| container::Style {
                background: Some(p.surface_container_highest.into()),
                border: iced::Border {
                    radius: 12.0.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            })
            .into();
        }
        // Something was just refused: the note says why, lit, and shakes
        // its head once. A refusal with nothing to see reads as a fault.
        // Outside a run it has no room of its own: a refusal is said
        // over the pane ([`Self::refusal_over`]), so nothing under it moves.
        if self.mode != Mode::Run {
            return Space::new().height(0).into();
        }
        self.note_line(false)
    }

    /// Why something was just refused, laid over the top of the pane
    /// outside a run. In the pane it pushed every row down as it came and
    /// let them back up as it went. Nothing, in a run or with nothing
    /// refused.
    pub(crate) fn refusal_over(&self) -> Element<'_, Message> {
        if self.mode == Mode::Run || self.motion.why().is_none() {
            return Space::new().height(0).into();
        }
        container(self.note_line(true)).padding([10, 12]).into()
    }

    /// The note's line: why something was refused, lit, shaking its head
    /// once, and Unlock in a run. `over` what is under it, its ground
    /// hides that; in its own room it is clear at rest.
    fn note_line(&self, over: bool) -> Element<'_, Message> {
        let p = self.palette;
        let refused = self.motion.why();
        let at_rest = p.on_surface_variant;
        let words = match refused {
            Some((why, _)) => capitalised(why),
            None => String::new(),
        };
        let lit = |rest: iced::Color, lit: iced::Color| match refused {
            Some((_, Some(faded))) => crate::motion::blend(lit, rest, faded * faded),
            Some((_, None)) => lit,
            None => rest,
        };
        let colour = lit(at_rest, p.tertiary);
        let ground = match over {
            true => lit(
                p.surface_container_highest,
                crate::motion::blend(p.surface_container_highest, p.tertiary, 0.3),
            ),
            false => lit(
                iced::Color::TRANSPARENT,
                crate::palette::Palette::tint(p.tertiary, 0.3),
            ),
        };
        let aside = self.motion.nudge();
        let mut line = row![
            container(
                text(words)
                    .size(META)
                    .color(colour)
                    .wrapping(text::Wrapping::None)
            )
            .width(Fill)
            .clip(true)
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if self.mode == Mode::Run {
            let (label, does, tone) = match self.unlocked {
                Some(_) => ("Done", "Lock the plan again", Tone::Solid),
                None => ("Unlock", "Change what is still ahead", Tone::Selected),
            };
            line = line.push(hinted(
                p,
                button(text(label).size(META).font(BOLD))
                    .padding([4, 10])
                    .on_press(Message::Key {
                        op: Op::Unlock,
                        only_if_empty: false,
                    })
                    .style(look::pill(p, tone)),
                keys::with_key(does, Op::Unlock),
                tooltip::Position::Bottom,
            ));
        }
        container(line)
            .padding(
                iced::Padding::default()
                    .top(6.0)
                    .bottom(6.0)
                    .left(10.0 + aside)
                    .right(10.0 - aside),
            )
            .style(move |_| container::Style {
                background: Some(ground.into()),
                border: iced::Border {
                    radius: 12.0.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            })
            .into()
    }

    /// The pane in Live, where the plan would be: what was shown this
    /// session, newest last, and the way to make a plan of it.
    pub(crate) fn session_pane(&self) -> Element<'_, Message> {
        let p = self.palette;
        let shown: Vec<_> = self
            .engine
            .0
            .log()
            .into_iter()
            .skip(self.live_from)
            .collect();
        let on = self.stage.program.as_ref().map(|scene| &scene.resource.id);
        let mut rows = column![].spacing(2);
        for entry in &shown {
            let live = on == Some(&entry.resource.id);
            let colour = if live { p.tertiary } else { p.on_surface };
            rows = rows.push(
                row![
                    text(
                        chrono::DateTime::<chrono::Local>::from(entry.at)
                            .format("%H:%M")
                            .to_string()
                    )
                    .size(META)
                    .color(p.on_surface_variant)
                    .width(44),
                    look::icon(Icon::of(&entry.resource.kind), 13.0, colour),
                    container(
                        single_line(entry.resource.title.clone())
                            .size(BODY)
                            .color(colour)
                    )
                    .width(Fill)
                    .clip(true),
                ]
                .spacing(8)
                .padding([6, 8])
                .align_y(Alignment::Center),
            );
        }
        let body: Element<'_, Message> = if shown.is_empty() {
            Space::new().into()
        } else {
            scrollable(rows).height(Fill).into()
        };
        let count = shown.len();
        let pane = column![
            self.mode_note(),
            row![
                text("Shown this session")
                    .size(META)
                    .font(BOLD)
                    .color(p.on_surface_variant),
                Space::new().width(Fill),
                chip(p, &count.to_string(), Tone::Plain),
            ]
            .padding([4, 8])
            .align_y(Alignment::Center),
            container(body).height(Fill),
            pill(
                p,
                "Save these as a plan",
                Tone::Selected,
                PILL,
                (count > 0).then_some(Message::SaveSession),
            )
            .width(Fill),
        ]
        .spacing(8)
        .padding([16, 12])
        .height(Fill);
        iced::widget::stack![pane, self.refusal_over()]
            .width(Fill)
            .height(Fill)
            .into()
    }
}
