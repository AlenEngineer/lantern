//! The keyboard. Typing goes to the search box while the search is open,
//! so every shortcut is a key that doesn't type: Ctrl combinations, Enter,
//! Tab, the arrows, Page Up/Down (what presentation clickers send), Escape
//! and media keys.
//!
//! One table, [`ACTIONS`], names what keys do and gives each its key; the
//! config's `[keys]` table can give any of them another ([`Keymap`]). A
//! script's `press NAME` goes by the table's own keys, so a script presses
//! what a person would with no config.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use iced::event;
use iced::keyboard::{self, Key, key::Named};

use crate::Message;

/// What the operator can do from a key, a button or a script's `press`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    /// Completes a half-typed bang, searches an on-request bang, previews
    /// the first result, or takes the preview: whichever comes first.
    Enter,
    /// Puts the preview on stage.
    Take,
    /// Adds the preview to the plan after the current step.
    AddToPlan,
    /// Puts the result, or what's lined up, on the plan's standby shelf.
    Standby,
    Next,
    Prev,
    PlayPause,
    Blackout,
    /// Empties the stage.
    Clear,
    PickNext,
    PickPrev,
    /// Empties the search box, keeping it focused.
    ClearSearch,
    /// Saves the plan to its file, or a new one named after it.
    Save,
    /// Shows or hides the list of saved plans.
    Open,
    /// Starts an empty plan.
    New,
    /// Completes a half-typed bang (`!y` → `!yt `).
    Complete,
    /// Moves the selected step up or down.
    MoveUp,
    MoveDown,
    /// Takes the bang chip off the search, so it asks every source again.
    DropBang,
    /// Opens or closes the settings.
    Settings,
    /// Opens the search, with the keyboard in its box.
    Search,
    /// Removes the selected step.
    RemoveStep,
    /// Edits the selected step's name, in its row.
    Rename,
    /// Takes back the last edit of the plan.
    Undo,
}

/// A key with its modifiers, as the config writes it (`ctrl+n`, `alt+up`,
/// `pagedown`) and as a button shows it (`Ctrl+N`, `Alt+↑`, `PgDn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bind {
    ctrl: bool,
    alt: bool,
    shift: bool,
    key: Pressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pressed {
    Char(char),
    Named(Named),
}

/// The keys with a name: how the config writes each, and how a label does.
const NAMED: &[(&str, Named, &str)] = &[
    ("enter", Named::Enter, "Enter"),
    ("tab", Named::Tab, "Tab"),
    ("up", Named::ArrowUp, "↑"),
    ("down", Named::ArrowDown, "↓"),
    ("left", Named::ArrowLeft, "←"),
    ("right", Named::ArrowRight, "→"),
    ("pageup", Named::PageUp, "PgUp"),
    ("pagedown", Named::PageDown, "PgDn"),
    ("escape", Named::Escape, "Esc"),
    ("backspace", Named::Backspace, "Backspace"),
    ("delete", Named::Delete, "Del"),
    ("space", Named::Space, "Space"),
    ("home", Named::Home, "Home"),
    ("end", Named::End, "End"),
    ("insert", Named::Insert, "Ins"),
    ("playpause", Named::MediaPlayPause, "Play"),
    ("f1", Named::F1, "F1"),
    ("f2", Named::F2, "F2"),
    ("f3", Named::F3, "F3"),
    ("f4", Named::F4, "F4"),
    ("f5", Named::F5, "F5"),
    ("f6", Named::F6, "F6"),
    ("f7", Named::F7, "F7"),
    ("f8", Named::F8, "F8"),
    ("f9", Named::F9, "F9"),
    ("f10", Named::F10, "F10"),
    ("f11", Named::F11, "F11"),
    ("f12", Named::F12, "F12"),
];

impl FromStr for Bind {
    type Err = String;

    /// `ctrl+n`, `alt+up`, `ctrl+shift+s`, `pagedown`. A letter alone is
    /// refused: it would type.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let wrong = || {
            format!(
                "“{}” isn't a key like ctrl+n, alt+up or pagedown",
                text.trim()
            )
        };
        let lower = text.trim().to_lowercase();
        // `ctrl++` and a lone `+` end in the key `+`.
        let (mods, key) = match lower.strip_suffix('+') {
            Some(mods) if !lower.is_empty() => (mods.trim_end_matches('+'), "+"),
            _ => match lower.rsplit_once('+') {
                Some((mods, key)) => (mods, key),
                None => ("", lower.as_str()),
            },
        };
        let mut bind = Bind {
            ctrl: false,
            alt: false,
            shift: false,
            key: Pressed::Char(' '),
        };
        for word in mods.split('+').filter(|m| !m.is_empty()) {
            match word.trim() {
                "ctrl" | "control" | "cmd" => bind.ctrl = true,
                "alt" => bind.alt = true,
                "shift" => bind.shift = true,
                _ => return Err(wrong()),
            }
        }
        let key = key.trim();
        let mut chars = key.chars();
        bind.key = match (chars.next(), chars.next()) {
            (Some(c), None) => Pressed::Char(c),
            _ => match NAMED.iter().find(|(name, ..)| *name == key) {
                Some((_, named, _)) => Pressed::Named(*named),
                None if key == "esc" => Pressed::Named(Named::Escape),
                None => return Err(wrong()),
            },
        };
        if matches!(bind.key, Pressed::Char(_)) && !bind.ctrl && !bind.alt {
            return Err(format!(
                "“{}” alone would type: add ctrl or alt",
                text.trim()
            ));
        }
        Ok(bind)
    }
}

impl Bind {
    fn parts(&self, ctrl: &str, alt: &str, shift: &str, key: String) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push(ctrl.to_string());
        }
        if self.alt {
            parts.push(alt.to_string());
        }
        if self.shift {
            parts.push(shift.to_string());
        }
        parts.push(key);
        parts.join("+")
    }

    fn named(named: Named) -> (&'static str, &'static str) {
        NAMED
            .iter()
            .find(|(_, n, _)| *n == named)
            .map_or(("?", "?"), |(written, _, label)| (*written, *label))
    }

    /// As the config writes it: `ctrl+n`.
    pub fn written(&self) -> String {
        let key = match self.key {
            Pressed::Char(c) => c.to_string(),
            Pressed::Named(named) => Bind::named(named).0.to_string(),
        };
        self.parts("ctrl", "alt", "shift", key)
    }
}

impl fmt::Display for Bind {
    /// As a button shows it: `Ctrl+N`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key = match self.key {
            Pressed::Char(c) => c.to_uppercase().to_string(),
            Pressed::Named(named) => Bind::named(named).1.to_string(),
        };
        f.write_str(&self.parts("Ctrl", "Alt", "Shift", key))
    }
}

/// When a key counts while the search box has focus, which is nearly always.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum When {
    /// Only if the box didn't use it: Enter submits the box itself, and
    /// Tab, the up and down arrows and Page Up/Down pass through.
    Unused,
    /// Even if the box used it: Ctrl keys, Escape, media keys.
    Always,
    /// Only while the box is empty: left and right move the cursor
    /// otherwise.
    EmptySearch,
}

/// Something a key does: its name in the config's `[keys]` table, the key
/// it has unless the config says otherwise (also what a script's `press`
/// calls it), and what it does, for the settings.
pub struct Action {
    pub name: &'static str,
    pub key: &'static str,
    pub op: Op,
    when: When,
    pub about: &'static str,
}

const fn action(
    name: &'static str,
    key: &'static str,
    op: Op,
    when: When,
    about: &'static str,
) -> Action {
    Action {
        name,
        key,
        op,
        when,
        about,
    }
}

/// Every key.
pub const ACTIONS: &[Action] = &[
    action(
        "show",
        "enter",
        Op::Enter,
        When::Unused,
        "Look at the top result, again to show it; show the selected step; open the picked plan",
    ),
    action(
        "complete",
        "tab",
        Op::Complete,
        When::Unused,
        "Complete a half-typed bang",
    ),
    action(
        "pick-next",
        "down",
        Op::PickNext,
        When::Unused,
        "The next result; without a search, the next step or plan",
    ),
    action(
        "pick-prev",
        "up",
        Op::PickPrev,
        When::Unused,
        "The result before; without a search, the step or plan before",
    ),
    action(
        "next",
        "pagedown",
        Op::Next,
        When::Unused,
        "Next slide, page or plan step",
    ),
    action(
        "prev",
        "pageup",
        Op::Prev,
        When::Unused,
        "Back a slide, page or plan step",
    ),
    action(
        "next-arrow",
        "right",
        Op::Next,
        When::EmptySearch,
        "Next, while the search is empty",
    ),
    action(
        "prev-arrow",
        "left",
        Op::Prev,
        When::EmptySearch,
        "Back, while the search is empty",
    ),
    action(
        "close",
        "escape",
        Op::ClearSearch,
        When::Always,
        "Clear the search; close what's open",
    ),
    action(
        "drop-bang",
        "backspace",
        Op::DropBang,
        When::EmptySearch,
        "Take the bang chip off an empty search",
    ),
    action(
        "play-pause",
        "ctrl+space",
        Op::PlayPause,
        When::Always,
        "Play or pause",
    ),
    action(
        "play-pause-media",
        "playpause",
        Op::PlayPause,
        When::Always,
        "Play or pause (media key)",
    ),
    action("blackout", "ctrl+b", Op::Blackout, When::Always, "Blackout"),
    action(
        "add-to-plan",
        "ctrl+enter",
        Op::AddToPlan,
        When::Always,
        "Add the result to the plan",
    ),
    action(
        "standby",
        "alt+enter",
        Op::Standby,
        When::Always,
        "Keep the result on standby: ready, but not in the plan's order",
    ),
    action("save", "ctrl+s", Op::Save, When::Always, "Save the plan"),
    action("plans", "ctrl+o", Op::Open, When::Always, "Your plans"),
    action(
        "new",
        "ctrl+n",
        Op::New,
        When::Always,
        "New plan; in a plan, a new step",
    ),
    action(
        "undo",
        "ctrl+z",
        Op::Undo,
        When::Always,
        "Undo the last edit of the plan",
    ),
    action(
        "move-up",
        "alt+up",
        Op::MoveUp,
        When::Always,
        "Move the selected step up",
    ),
    action(
        "move-down",
        "alt+down",
        Op::MoveDown,
        When::Always,
        "Move the selected step down",
    ),
    action("settings", "ctrl+,", Op::Settings, When::Always, "Settings"),
    action("search", "ctrl+k", Op::Search, When::Always, "Search"),
    action(
        "remove-step",
        "delete",
        Op::RemoveStep,
        When::Unused,
        "Remove the selected step",
    ),
    action(
        "rename",
        "f2",
        Op::Rename,
        When::Always,
        "Rename the selected step",
    ),
];

/// The op a script's `press NAME` stands for: NAME is the key an action
/// has before any config, so scripts don't depend on the operator's keys.
pub fn op_named(name: &str) -> Option<Op> {
    ACTIONS
        .iter()
        .find(|action| action.key.eq_ignore_ascii_case(name))
        .map(|action| action.op)
}

/// The key each of [`ACTIONS`] has, in the same order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap(Vec<Bind>);

impl Keymap {
    pub fn defaults() -> Keymap {
        Keymap(
            ACTIONS
                .iter()
                .map(|action| action.key.parse().expect("every default key reads"))
                .collect(),
        )
    }

    /// The defaults with `changed` over them: action name to key, as the
    /// config's `[keys]` table has them. Says what's wrong with the first
    /// entry that is: an unknown action, a key that doesn't read, or one
    /// two actions would share.
    pub fn with(changed: &BTreeMap<String, String>) -> Result<Keymap, String> {
        let mut map = Keymap::defaults();
        for (name, key) in changed {
            let at = ACTIONS
                .iter()
                .position(|action| action.name == name)
                .ok_or_else(|| format!("There's no key called “{name}”."))?;
            map.0[at] = key
                .parse()
                .map_err(|err| format!("The key for {name}: {err}."))?;
        }
        for (i, bind) in map.0.iter().enumerate() {
            if let Some(j) = map.0[..i].iter().position(|other| other == bind) {
                return Err(format!(
                    "{bind} can't be both {} and {}.",
                    ACTIONS[j].name, ACTIONS[i].name
                ));
            }
        }
        Ok(map)
    }

    /// What differs from the defaults, as the config keeps it.
    pub fn changed(&self) -> BTreeMap<String, String> {
        let defaults = Keymap::defaults();
        ACTIONS
            .iter()
            .zip(&self.0)
            .zip(&defaults.0)
            .filter(|((_, bind), default)| bind != default)
            .map(|((action, bind), _)| (action.name.to_string(), bind.written()))
            .collect()
    }

    /// The key of action `at` of [`ACTIONS`].
    pub fn bind(&self, at: usize) -> Bind {
        self.0[at]
    }

    /// The first key that does `op`, as a button shows it.
    pub fn label(&self, op: Op) -> String {
        ACTIONS
            .iter()
            .zip(&self.0)
            .find(|(action, _)| action.op == op)
            .map(|(_, bind)| bind.to_string())
            .unwrap_or_default()
    }

    fn lookup(&self, key: &Key, ctrl: bool, alt: bool, shift: bool) -> Option<(Op, When)> {
        let pressed = match key.as_ref() {
            Key::Named(named) => Pressed::Named(named),
            Key::Character(text) => {
                let mut chars = text.chars();
                match (chars.next(), chars.next()) {
                    (Some(' '), None) => Pressed::Named(Named::Space),
                    (Some(c), None) => Pressed::Char(c.to_lowercase().next().unwrap_or(c)),
                    _ => return None,
                }
            }
            _ => return None,
        };
        let matches = |bind: &Bind, strict: bool| {
            bind.key == pressed
                && bind.ctrl == ctrl
                && bind.alt == alt
                && if strict {
                    bind.shift && shift
                } else {
                    !bind.shift
                }
        };
        // A key bound with shift wins over the same key bound without.
        [true, false].into_iter().find_map(|strict| {
            ACTIONS
                .iter()
                .zip(&self.0)
                .find(|(_, bind)| matches(bind, strict))
                .map(|(action, _)| (action.op, action.when))
        })
    }
}

/// The keys in use: the defaults until [`configure`] says otherwise. One
/// for the whole app, since the key listener and every label read it.
static KEYMAP: RwLock<Option<Keymap>> = RwLock::new(None);

fn keymap() -> Keymap {
    let map = KEYMAP.read().unwrap_or_else(|e| e.into_inner());
    map.clone().unwrap_or_else(Keymap::defaults)
}

/// Uses the config's `[keys]`. On an entry that's wrong, says why and
/// keeps the keys as they were.
pub fn configure(changed: &BTreeMap<String, String>) -> Result<(), String> {
    let map = Keymap::with(changed)?;
    *KEYMAP.write().unwrap_or_else(|e| e.into_inner()) = Some(map);
    Ok(())
}

/// Whether the next key pressed is being recorded for the settings, not
/// acted on.
static RECORDING: AtomicBool = AtomicBool::new(false);

/// Starts or stops recording: while on, a key press becomes
/// [`Message::KeyRecorded`] with the key as the config writes it, and
/// Escape alone gives it up.
pub fn record(on: bool) {
    RECORDING.store(on, Ordering::Relaxed);
}

/// A key press as the config writes it (`ctrl+shift+n`), or `None` for a
/// key that has no name here, such as a modifier on its own.
fn as_written(key: &Key, modifiers: keyboard::Modifiers) -> Option<String> {
    let key = match key.as_ref() {
        Key::Named(named) => NAMED.iter().find(|(_, n, _)| *n == named)?.0.to_string(),
        Key::Character(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(' '), None) => "space".to_string(),
                (Some(c), None) => c.to_lowercase().to_string(),
                _ => return None,
            }
        }
        _ => return None,
    };
    let mut parts = Vec::new();
    if modifiers.command() {
        parts.push("ctrl");
    }
    if modifiers.alt() {
        parts.push("alt");
    }
    if modifiers.shift() {
        parts.push("shift");
    }
    parts.push(&key);
    Some(parts.join("+"))
}

/// A key as a button shows it, from how the config writes it; as written
/// when it doesn't read.
pub fn shown(written: &str) -> String {
    written
        .parse::<Bind>()
        .map_or_else(|_| written.to_string(), |bind| bind.to_string())
}

/// The key that does `op`, as a button shows it: "Ctrl+N".
pub fn label(op: Op) -> String {
    keymap().label(op)
}

/// The key of the action called `name` in [`ACTIONS`], as a button shows
/// it: for an op with two keys, where the second is meant.
pub fn label_of(name: &str) -> String {
    ACTIONS
        .iter()
        .position(|action| action.name == name)
        .map(|at| keymap().bind(at).to_string())
        .unwrap_or_default()
}

/// "New plan · Ctrl+N": what a button does and its key.
pub fn with_key(what: &str, op: Op) -> String {
    format!("{what} · {}", label(op))
}

/// A key press as a message: an op, or text typed while the search box
/// didn't have focus, which goes into it. Held keys don't repeat (except
/// picking), so holding Ctrl+B can't flicker the projector.
pub fn key_event(
    event: iced::Event,
    status: event::Status,
    _: iced::window::Id,
) -> Option<Message> {
    let iced::Event::Keyboard(keyboard::Event::KeyPressed {
        key,
        modifiers,
        repeat,
        text,
        ..
    }) = event
    else {
        return None;
    };
    if RECORDING.load(Ordering::Relaxed) {
        if repeat {
            return None;
        }
        let escape = key == Key::Named(Named::Escape) && modifiers.is_empty();
        return match as_written(&key, modifiers) {
            _ if escape => Some(Message::KeyRecorded(None)),
            Some(written) => Some(Message::KeyRecorded(Some(written))),
            None => None,
        };
    }
    let used = status == event::Status::Captured;
    let ctrl = modifiers.command();
    let alt = modifiers.alt();
    if let Some((op, when)) = keymap().lookup(&key, ctrl, alt, modifiers.shift()) {
        if when == When::Unused && used {
            return None;
        }
        let repeats = matches!(op, Op::PickNext | Op::PickPrev);
        return (repeats || !repeat).then_some(Message::Key {
            op,
            only_if_empty: when == When::EmptySearch,
        });
    }
    let typed = text.filter(|text| !text.chars().any(char::is_control))?;
    (!used && !ctrl && !alt).then(|| Message::Type(typed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: Key, modifiers: keyboard::Modifiers, status: event::Status) -> Option<Message> {
        let text = match &key {
            Key::Character(c) => Some(c.clone()),
            _ => None,
        };
        let event = iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            text,
            repeat: false,
        });
        key_event(event, status, iced::window::Id::unique())
    }

    fn op(message: Option<Message>) -> Option<(Op, bool)> {
        match message {
            Some(Message::Key { op, only_if_empty }) => Some((op, only_if_empty)),
            _ => None,
        }
    }

    const NONE: keyboard::Modifiers = keyboard::Modifiers::empty();
    const CTRL: keyboard::Modifiers = keyboard::Modifiers::CTRL;
    const TYPING: event::Status = event::Status::Captured;
    const FREE: event::Status = event::Status::Ignored;

    #[test]
    fn ctrl_keys_work_while_typing() {
        let ctrl = |c: &str| op(press(Key::Character(c.into()), CTRL, TYPING));
        assert_eq!(ctrl("b"), Some((Op::Blackout, false)));
        // Ctrl+A is the search box's own: it selects what's typed.
        assert_eq!(ctrl("A"), None);
        assert_eq!(
            op(press(Key::Named(Named::Enter), CTRL, TYPING)),
            Some((Op::AddToPlan, false))
        );
        assert_eq!(ctrl("s"), Some((Op::Save, false)));
        assert_eq!(ctrl("q"), None);
        // Alt+Enter is standby; Ctrl+Enter, the plan.
        assert_eq!(
            op(press(
                Key::Named(Named::Enter),
                keyboard::Modifiers::ALT,
                TYPING
            )),
            Some((Op::Standby, false))
        );
        assert_eq!(
            op(press(Key::Named(Named::Space), CTRL, TYPING)),
            Some((Op::PlayPause, false))
        );
        assert_eq!(
            op(press(
                Key::Named(Named::ArrowUp),
                keyboard::Modifiers::ALT,
                TYPING
            )),
            Some((Op::MoveUp, false))
        );
    }

    #[test]
    fn letters_and_space_type_instead_of_acting() {
        assert!(
            matches!(press(Key::Character("b".into()), NONE, FREE), Some(Message::Type(t)) if t == "b")
        );
        assert!(
            press(Key::Character("b".into()), NONE, TYPING).is_none(),
            "the box typed it"
        );
        assert!(op(press(Key::Named(Named::Space), NONE, FREE)).is_none());
    }

    #[test]
    fn arrows_step_only_through_an_empty_search() {
        assert_eq!(
            op(press(Key::Named(Named::ArrowRight), NONE, TYPING)),
            Some((Op::Next, true))
        );
        assert_eq!(
            op(press(Key::Named(Named::PageDown), NONE, FREE)),
            Some((Op::Next, false))
        );
        assert!(
            press(Key::Named(Named::Enter), NONE, TYPING).is_none(),
            "the box submits"
        );
        assert_eq!(
            op(press(Key::Named(Named::Escape), NONE, TYPING)),
            Some((Op::ClearSearch, false))
        );
        assert_eq!(
            op(press(Key::Named(Named::Backspace), NONE, TYPING)),
            Some((Op::DropBang, true)),
            "only an empty box gives up its chip"
        );
    }

    #[test]
    fn script_key_names_match_the_table() {
        assert_eq!(op_named("Enter"), Some(Op::Enter));
        assert_eq!(op_named("pagedown"), Some(Op::Next));
        assert_eq!(op_named("Ctrl+B"), Some(Op::Blackout));
        assert_eq!(op_named("ctrl+space"), Some(Op::PlayPause));
        assert_eq!(op_named("b"), None);
    }

    #[test]
    fn a_pressed_key_is_written_as_the_config_would() {
        let shift = keyboard::Modifiers::SHIFT;
        let written = |key: Key, modifiers| as_written(&key, modifiers);
        assert_eq!(
            written(Key::Character("t".into()), CTRL).as_deref(),
            Some("ctrl+t")
        );
        assert_eq!(
            written(Key::Character("N".into()), CTRL | shift).as_deref(),
            Some("ctrl+shift+n")
        );
        assert_eq!(written(Key::Named(Named::F9), NONE).as_deref(), Some("f9"));
        assert_eq!(
            written(Key::Named(Named::Space), CTRL).as_deref(),
            Some("ctrl+space")
        );
        assert_eq!(
            written(Key::Named(Named::Control), CTRL),
            None,
            "a modifier alone"
        );
        assert_eq!(shown("ctrl+shift+n"), "Ctrl+Shift+N");
        assert_eq!(shown("nonsense"), "nonsense");
    }

    #[test]
    fn keys_read_as_written_and_show_as_labels() {
        let label = |text: &str| text.parse::<Bind>().map(|bind| bind.to_string());
        assert_eq!(label("ctrl+n").as_deref(), Ok("Ctrl+N"));
        assert_eq!(label(" Alt+Up ").as_deref(), Ok("Alt+↑"));
        assert_eq!(label("pagedown").as_deref(), Ok("PgDn"));
        assert_eq!(label("ctrl+shift+s").as_deref(), Ok("Ctrl+Shift+S"));
        assert_eq!(label("ctrl+,").as_deref(), Ok("Ctrl+,"));
        assert_eq!(label("ctrl++").as_deref(), Ok("Ctrl++"));
        assert_eq!("esc".parse::<Bind>().unwrap().written(), "escape");
        assert!(label("k").unwrap_err().contains("would type"));
        assert!(label("ctrl+banana").is_err());
        assert!(label("hyper+k").is_err());
        for action in ACTIONS {
            let bind: Bind = action.key.parse().unwrap();
            assert_eq!(bind.written(), action.key, "{}", action.name);
        }
    }

    #[test]
    fn the_config_changes_keys_and_says_what_is_wrong() {
        let with = |pairs: &[(&str, &str)]| {
            Keymap::with(
                &pairs
                    .iter()
                    .map(|(name, key)| (name.to_string(), key.to_string()))
                    .collect(),
            )
        };
        assert_eq!(with(&[]).unwrap(), Keymap::defaults());
        let map = with(&[("new", "ctrl+t"), ("blackout", "f9")]).unwrap();
        assert_eq!(map.label(Op::New), "Ctrl+T");
        assert_eq!(map.label(Op::Blackout), "F9");
        let ctrl = |c: &str| map.lookup(&Key::Character(c.into()), true, false, false);
        assert_eq!(ctrl("t").map(|(op, _)| op), Some(Op::New));
        assert_eq!(ctrl("n"), None, "the old key is free");
        assert_eq!(
            map.changed(),
            [("blackout", "f9"), ("new", "ctrl+t")]
                .into_iter()
                .map(|(name, key)| (name.to_string(), key.to_string()))
                .collect()
        );
        assert!(
            with(&[("fly", "ctrl+f")])
                .unwrap_err()
                .contains("no key called")
        );
        assert!(with(&[("new", "n")]).unwrap_err().contains("would type"));
        let clash = with(&[("new", "ctrl+s")]).unwrap_err();
        assert!(clash.contains("save") && clash.contains("new"), "{clash}");
        // Shift makes a different key of the same letter.
        let map = with(&[("plans", "ctrl+shift+n")]).unwrap();
        let n = |shift| map.lookup(&Key::Character("N".into()), true, false, shift);
        assert_eq!(n(true).map(|(op, _)| op), Some(Op::Open));
        assert_eq!(n(false).map(|(op, _)| op), Some(Op::New));
    }
}
