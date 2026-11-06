//! Scripted mode: drives the engine from a file, one command per line, so
//! `scripts/e2e.sh` can run the real app and screenshot it.
//!
//! To add a command, add a variant to [`Command`], a line to
//! [`Command::parse`] and an arm to [`Script::perform`].

use std::fmt;
use std::path::{Path, PathBuf};
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
    /// `show PATH`: show a local file now, off-plan. A relative path is
    /// relative to the script's folder.
    Show(PathBuf),
    /// `preview PATH`: line a local file up in preview.
    Preview(PathBuf),
    /// `add PATH`: add a local file to the plan after the current step.
    Add(PathBuf),
    /// `take`: put the preview on stage.
    Take,
    /// `play`: play the program.
    Play,
    /// `pause`: pause the program.
    Pause,
    /// `seek SECS`: jump the program's timeline.
    Seek(Duration),
    /// `frame N`: go to page or stanza N, counted from 1.
    Frame(usize),
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
    fn parse(line: &str, dir: &Path) -> Result<Command, String> {
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
            "show" | "preview" | "add" if rest.is_empty() => {
                return Err(format!("{word} needs a path"));
            }
            "show" => Command::Show(dir.join(rest)),
            "preview" => Command::Preview(dir.join(rest)),
            "add" => Command::Add(dir.join(rest)),
            "take" => Command::Take,
            "play" => Command::Play,
            "pause" => Command::Pause,
            "seek" => {
                let secs: f64 = rest
                    .parse()
                    .map_err(|_| format!("seek needs seconds, got {rest:?}"))?;
                Command::Seek(
                    Duration::try_from_secs_f64(secs)
                        .map_err(|_| format!("seek needs seconds, got {rest:?}"))?,
                )
            }
            "frame" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Command::Frame(n - 1),
                _ => return Err(format!("frame needs a number from 1, got {rest:?}")),
            },
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
            Command::Wait(_)
                | Command::Text(_)
                | Command::Shot(_)
                | Command::Show(_)
                | Command::Preview(_)
                | Command::Add(_)
                | Command::Seek(_)
                | Command::Frame(_)
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
    /// Blank lines and lines starting with `#` are skipped. Relative paths
    /// are resolved against `dir`, the script's folder.
    pub fn parse(source: &str, dir: &Path, shooter: Option<PathBuf>) -> Result<Script, ParseError> {
        let commands = source
            .lines()
            .enumerate()
            .map(|(i, line)| (i + 1, line.trim()))
            .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
            .map(|(line, text)| {
                Command::parse(text, dir).map_err(|message| ParseError { line, message })
            })
            .collect::<Result<_, _>>()?;
        Ok(Script { commands, shooter })
    }

    /// Runs every command in order, blocking, until the end or `quit`.
    /// Calls `acted` after each one.
    pub fn run(&self, engine: &Engine, mut acted: impl FnMut()) {
        for command in &self.commands {
            log(format!("script: {command:?}"));
            if *command == Command::Quit {
                return;
            }
            self.perform(command, engine);
            acted();
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
            Command::Show(path) => engine.show_now(Arc::new(file(path))),
            Command::Preview(path) => engine.preview(Arc::new(file(path))),
            Command::Add(path) => {
                engine.add_after_current(&file(path));
            }
            Command::Take => log(format!("script: take -> {}", engine.take())),
            Command::Play => log(format!("script: play -> {}", engine.play())),
            Command::Pause => log(format!("script: pause -> {}", engine.pause())),
            Command::Seek(position) => log(format!("script: seek -> {}", engine.seek(*position))),
            Command::Frame(frame) => {
                log(format!("script: frame -> {}", engine.goto_frame(*frame)));
            }
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

/// A local file as a resource, its kind guessed from the extension.
fn file(path: &Path) -> Resource {
    let source = SourceId::new("script").expect("a valid source id");
    let kind = path
        .extension()
        .and_then(|ext| Kind::from_extension(&ext.to_string_lossy()))
        .unwrap_or_else(|| Kind::Other("unknown".into()));
    let title = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Resource::new(
        ResourceId::new(&source, &path.display().to_string()),
        title,
        kind,
        Location::File(path.to_path_buf()),
    )
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
        Script::parse(source, Path::new("/media"), None)
            .unwrap()
            .run(&engine, || {});
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
        let script =
            Script::parse("# setup\n\n  wait 5\nshot first\n", Path::new(""), None).unwrap();
        assert_eq!(
            script.commands,
            [
                Command::Wait(Duration::from_millis(5)),
                Command::Shot("first".into())
            ]
        );
    }

    #[test]
    fn media_commands_drive_the_stage() {
        let engine = run("show pictures/a.png");
        let program = engine.stage().program.unwrap();
        assert_eq!(program.resource.kind, Kind::Image);
        assert_eq!(
            program.resource.location,
            Location::File("/media/pictures/a.png".into())
        );

        let engine = run("preview clip.mp4\ntake\nplay\nseek 1.5");
        let timed = *engine.stage().program.unwrap().timed().unwrap();
        assert_eq!(timed.playback, chorquest_engine::Playback::Playing);
        assert_eq!(timed.position, Duration::from_millis(1500));

        let engine = run("add a.png\nnext");
        assert_eq!(engine.stage().program.unwrap().resource.title, "a.png");
    }

    #[test]
    fn frame_counts_from_one() {
        let engine = run("hymn\nframe 3");
        assert!(
            on_stage(&engine)
                .unwrap()
                .starts_with("Through many dangers")
        );
    }

    #[test]
    fn errors_name_the_line() {
        let err = |source| {
            Script::parse(source, Path::new("/media"), None)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(err("hymn\njump"), r#"line 2: unknown command "jump""#);
        assert_eq!(
            err("wait soon"),
            r#"line 1: wait needs milliseconds, got "soon""#
        );
        assert_eq!(err("next 3"), "line 1: next takes no argument");
        assert_eq!(err("show"), "line 1: show needs a path");
        assert_eq!(
            err("frame 0"),
            r#"line 1: frame needs a number from 1, got "0""#
        );
        assert_eq!(err("seek -1"), r#"line 1: seek needs seconds, got "-1""#);
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
