//! The keyboard. Typing always goes to the search box, so every shortcut
//! is a key that doesn't type: Ctrl combinations, Enter, Tab, the arrows,
//! Page Up/Down (what presentation clickers send), Escape and media keys.
//!
//! One table, [`KEYS`], maps keys to [`Op`]s for the keyboard and for a
//! script's `press NAME`, so a script presses exactly what a person would.

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
    /// Switches between live and plan mode.
    ToggleMode,
    /// Completes a half-typed bang (`!y` → `!yt `).
    Complete,
    /// Edit mode on or off: clicks in the plan edit it instead of running it.
    ToggleEditing,
    /// Takes the bang chip off the search, so it asks every source again.
    DropBang,
}

enum Bind {
    Named(Named),
    Ctrl(char),
    CtrlNamed(Named),
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

/// Every key, by the name a script's `press` uses.
const KEYS: &[(&str, Bind, Op, When)] = &[
    ("enter", Bind::Named(Named::Enter), Op::Enter, When::Unused),
    ("tab", Bind::Named(Named::Tab), Op::Complete, When::Unused),
    (
        "down",
        Bind::Named(Named::ArrowDown),
        Op::PickNext,
        When::Unused,
    ),
    (
        "up",
        Bind::Named(Named::ArrowUp),
        Op::PickPrev,
        When::Unused,
    ),
    (
        "pagedown",
        Bind::Named(Named::PageDown),
        Op::Next,
        When::Unused,
    ),
    ("pageup", Bind::Named(Named::PageUp), Op::Prev, When::Unused),
    (
        "right",
        Bind::Named(Named::ArrowRight),
        Op::Next,
        When::EmptySearch,
    ),
    (
        "left",
        Bind::Named(Named::ArrowLeft),
        Op::Prev,
        When::EmptySearch,
    ),
    (
        "escape",
        Bind::Named(Named::Escape),
        Op::ClearSearch,
        When::Always,
    ),
    (
        "backspace",
        Bind::Named(Named::Backspace),
        Op::DropBang,
        When::EmptySearch,
    ),
    (
        "ctrl+space",
        Bind::CtrlNamed(Named::Space),
        Op::PlayPause,
        When::Always,
    ),
    (
        "playpause",
        Bind::Named(Named::MediaPlayPause),
        Op::PlayPause,
        When::Always,
    ),
    ("ctrl+b", Bind::Ctrl('b'), Op::Blackout, When::Always),
    ("ctrl+a", Bind::Ctrl('a'), Op::AddToPlan, When::Always),
    ("ctrl+s", Bind::Ctrl('s'), Op::Save, When::Always),
    ("ctrl+o", Bind::Ctrl('o'), Op::Open, When::Always),
    ("ctrl+n", Bind::Ctrl('n'), Op::New, When::Always),
    ("ctrl+p", Bind::Ctrl('p'), Op::ToggleMode, When::Always),
    ("ctrl+e", Bind::Ctrl('e'), Op::ToggleEditing, When::Always),
];

/// The op a script's `press NAME` stands for.
pub fn op_named(name: &str) -> Option<Op> {
    KEYS.iter()
        .find(|(n, ..)| n.eq_ignore_ascii_case(name))
        .map(|(_, _, op, _)| *op)
}

fn lookup(key: &Key, ctrl: bool) -> Option<(Op, When)> {
    let is = |pressed: &str, c: char| pressed.to_lowercase().chars().eq(std::iter::once(c));
    KEYS.iter()
        .find(|(_, bind, ..)| match (bind, key.as_ref()) {
            (Bind::Ctrl(c), Key::Character(pressed)) => ctrl && is(pressed, *c),
            (Bind::CtrlNamed(named), Key::Named(pressed)) => ctrl && *named == pressed,
            (Bind::CtrlNamed(Named::Space), Key::Character(" ")) => ctrl,
            (Bind::Named(named), Key::Named(pressed)) => !ctrl && *named == pressed,
            _ => false,
        })
        .map(|(_, _, op, when)| (*op, *when))
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
    let used = status == event::Status::Captured;
    let ctrl = modifiers.command();
    if modifiers.alt() {
        return None;
    }
    if let Some((op, when)) = lookup(&key, ctrl) {
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
    (!used && !ctrl).then(|| Message::Type(typed.to_string()))
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
        assert_eq!(ctrl("A"), Some((Op::AddToPlan, false)));
        assert_eq!(ctrl("s"), Some((Op::Save, false)));
        assert_eq!(ctrl("q"), None);
        assert_eq!(
            op(press(Key::Named(Named::Space), CTRL, TYPING)),
            Some((Op::PlayPause, false))
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
}
