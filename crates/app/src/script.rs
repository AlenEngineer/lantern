//! Scripted mode: drives the engine from a file, one command per line, so
//! `scripts/e2e.sh` can run the real app and screenshot it.
//!
//! To add a command, add a variant to [`Command`], a line to
//! [`Command::parse`] and an arm to [`Script::perform`].

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chorquest_engine::{Engine, Kind, Location, Resource, ResourceId, SourceId};

use crate::log;

/// One line of a script.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Command {
    /// `wait MS`: sleep.
    Wait(Duration),
    /// `hymn`: add the built-in hymn to the plan and go to it.
    Hymn,
    /// `text "..."`: show inline text now, off-plan. `\n` starts a new line.
    Text(String),
    /// `next`: the next button.
    Next,
    /// `prev`: the back button.
    Prev,
    /// `black`: flip blackout.
    Black,
    /// `clear`: empty the stage.
    Clear,
    /// `shot NAME`: ask the harness to capture both windows.
    Shot(String),
    /// `quit`: stop here.
    Quit,
}

/// A line the parser rejected.
#[derive(Debug, PartialEq, Eq)]
pub struct ParseError {
    line: usize,
    message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl Command {
    fn parse(line: &str) -> Result<Command, String> {
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        let command = match word {
            "wait" => {
                let ms = rest
                    .parse()
                    .map_err(|_| format!("wait needs milliseconds, got {rest:?}"))?;
                Command::Wait(Duration::from_millis(ms))
            }
            "hymn" => Command::Hymn,
            "text" => {
                let body = rest
                    .strip_prefix('"')
                    .and_then(|r| r.strip_suffix('"'))
                    .ok_or_else(|| format!("text needs a quoted string, got {rest:?}"))?;
                Command::Text(body.replace("\\n", "\n"))
            }
            "next" => Command::Next,
            "prev" => Command::Prev,
            "black" => Command::Black,
            "clear" => Command::Clear,
            "shot" if !rest.is_empty() && !rest.contains(['/', ' ']) => {
                Command::Shot(rest.to_string())
            }
            "shot" => return Err(format!("shot needs a plain name, got {rest:?}")),
            "quit" => Command::Quit,
            other => return Err(format!("unknown command {other:?}")),
        };
        let takes_argument = matches!(
            command,
            Command::Wait(_) | Command::Text(_) | Command::Shot(_)
        );
        if !takes_argument && !rest.is_empty() {
            return Err(format!("{word} takes no argument"));
        }
        Ok(command)
    }
}

/// A parsed script and how to ask for screenshots.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Script {
    commands: Vec<Command>,
    /// Run with the shot name as its only argument. It returns once both
    /// windows are captured. Without it, `shot` only logs.
    shooter: Option<PathBuf>,
}

impl Script {
    /// Parses the whole script up front, so a typo fails before any window opens.
    /// Blank lines and lines starting with `#` are skipped.
    pub fn parse(source: &str, shooter: Option<PathBuf>) -> Result<Script, ParseError> {
        let commands = source
            .lines()
            .enumerate()
            .map(|(i, line)| (i + 1, line.trim()))
            .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
            .map(|(line, text)| {
                Command::parse(text).map_err(|message| ParseError { line, message })
            })
            .collect::<Result<_, _>>()?;
        Ok(Script { commands, shooter })
    }

    /// Runs every command in order, blocking, until the end or `quit`.
    pub fn run(&self, engine: &Engine) {
        for command in &self.commands {
            log(format!("script: {command:?}"));
            if *command == Command::Quit {
                return;
            }
            self.perform(command, engine);
        }
    }

    fn perform(&self, command: &Command, engine: &Engine) {
        match command {
            Command::Wait(duration) => std::thread::sleep(*duration),
            Command::Hymn => {
                let step = engine.add_after_current(&hymn());
                if let Err(err) = engine.go_to(step) {
                    log(format!("script: hymn: {err}"));
                }
            }
            Command::Text(body) => engine.show_now(Arc::new(inline("text", "Text", body))),
            Command::Next => log(format!("script: next -> {:?}", engine.next())),
            Command::Prev => log(format!("script: prev -> {:?}", engine.prev())),
            Command::Black => {
                log(format!("script: blackout {}", engine.toggle_blackout()));
            }
            Command::Clear => engine.clear(),
            Command::Shot(name) => self.shoot(name),
            Command::Quit => {}
        }
    }

    fn shoot(&self, name: &str) {
        let Some(shooter) = &self.shooter else {
            log(format!(
                "script: shot {name} skipped, CHORQUEST_SHOT is unset"
            ));
            return;
        };
        match std::process::Command::new(shooter).arg(name).status() {
            Ok(status) if status.success() => log(format!("script: shot {name} saved")),
            Ok(status) => log(format!("script: shot {name} failed: {status}")),
            Err(err) => log(format!("script: shot {name}: {}: {err}", shooter.display())),
        }
    }
}

const HYMN: &str = "\
Amazing grace! How sweet the sound
That saved a wretch like me!
I once was lost, but now am found;
Was blind, but now I see.

'Twas grace that taught my heart to fear,
And grace my fears relieved;
How precious did that grace appear
The hour I first believed.

Through many dangers, toils and snares,
I have already come;
'Tis grace hath brought me safe thus far,
And grace will lead me home.";

fn hymn() -> Resource {
    inline("amazing-grace", "Amazing Grace", HYMN)
}

fn inline(key: &str, title: &str, body: &str) -> Resource {
    let source = SourceId::new("script").expect("a valid source id");
    Resource::new(
        ResourceId::new(&source, key),
        title,
        Kind::Text,
        Location::Inline(body.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Engine {
        let engine = Engine::new();
        Script::parse(source, None).unwrap().run(&engine);
        engine
    }

    fn on_stage(engine: &Engine) -> Option<String> {
        let state = engine.stage();
        if state.blackout {
            return None;
        }
        state.program.and_then(|scene| scene.text())
    }

    #[test]
    fn hymn_steps_through_its_stanzas() {
        let engine = run("hymn\nnext\nnext");
        assert!(
            on_stage(&engine)
                .unwrap()
                .starts_with("Through many dangers")
        );
        assert_eq!(engine.plan().steps().len(), 1, "the hymn is a plan step");
    }

    #[test]
    fn black_twice_shows_the_stanza_again_and_clear_empties() {
        let engine = run("hymn\nblack");
        assert_eq!(on_stage(&engine), None);
        let engine = run("hymn\nblack\nblack");
        assert!(on_stage(&engine).unwrap().starts_with("Amazing grace"));
        let engine = run("hymn\nclear");
        assert!(engine.stage().program.is_none());
    }

    #[test]
    fn text_shows_inline_text_with_line_breaks() {
        let engine = run(r#"text "Welcome\nto church""#);
        assert_eq!(on_stage(&engine).unwrap(), "Welcome\nto church");
    }

    #[test]
    fn quit_stops_the_script() {
        let engine = run("hymn\nquit\nnext");
        assert!(on_stage(&engine).unwrap().starts_with("Amazing grace"));
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let script = Script::parse("# setup\n\n  wait 5\nshot first\n", None).unwrap();
        assert_eq!(
            script.commands,
            [
                Command::Wait(Duration::from_millis(5)),
                Command::Shot("first".into())
            ]
        );
    }

    #[test]
    fn errors_name_the_line() {
        let err = |source| Script::parse(source, None).unwrap_err().to_string();
        assert_eq!(err("hymn\njump"), r#"line 2: unknown command "jump""#);
        assert_eq!(
            err("wait soon"),
            r#"line 1: wait needs milliseconds, got "soon""#
        );
        assert_eq!(err("next 3"), "line 1: next takes no argument");
        assert_eq!(
            err("text hello"),
            r#"line 1: text needs a quoted string, got "hello""#
        );
        assert_eq!(
            err("shot ../x"),
            r#"line 1: shot needs a plain name, got "../x""#
        );
    }
}
