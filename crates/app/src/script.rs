//! Scripted mode: drives the engine from a file, one command per line, so
//! `scripts/e2e.sh` can run the real app and screenshot it.
//!
//! To add a command, add a variant to [`Command`], a line to
//! [`Command::parse`] and an arm to [`Script::perform`].

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lantern_engine::{
    Engine, Hit, Kind, Location, Query, Resource, ResourceId, SearchEvent, SourceId,
};
use lantern_source_link::LinkSource;

use crate::keys::{self, Op};
use crate::library::Library;
use crate::log;
use crate::youtube::{Downloads, YoutubeVideo};

/// What `show`, `preview` and `add` put on stage.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    /// A local file. A relative path is relative to the script's folder.
    File(PathBuf),
    /// A link, resolved by the link source as if pasted into the search box.
    Link(String),
}

impl Target {
    fn parse(text: &str, dir: &Path) -> Result<Target, String> {
        if text.starts_with("http://") || text.starts_with("https://") {
            links()
                .resolve(text)
                .ok_or_else(|| format!("not a link: {text:?}"))?;
            return Ok(Target::Link(text.to_string()));
        }
        Ok(Target::File(dir.join(text)))
    }

    /// Starts downloading a YouTube link. The app does too, but only once
    /// its event loop catches up, and a following `wait-ready` must see it.
    fn resource(&self, downloads: &Arc<Downloads>) -> Resource {
        let resource = match self {
            Target::File(path) => file(path),
            Target::Link(url) => links().resolve(url).expect("checked by Target::parse"),
        };
        if let Some(video) = YoutubeVideo::of(&resource) {
            downloads.fetch(&video);
        }
        resource
    }
}

fn links() -> LinkSource {
    LinkSource::new(SourceId::new("link").expect("a valid source id"))
}

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
    /// `show PATH|URL`: show a file or link now, off-plan.
    Show(Target),
    /// `preview PATH|URL`: line a file or link up in preview.
    Preview(Target),
    /// `add PATH|URL`: add a file or link to the plan after the current step.
    Add(Target),
    /// `section BUDGET TITLE`: add a section with a budget (`15`, `0:20`)
    /// after the current step.
    Section(Duration, String),
    /// `slot SEARCH`: add a live slot that searches SEARCH (`!b jn 3`).
    Slot(String),
    /// `path TEXT`: type TEXT in the open settings' first folder path,
    /// adding a folder row if there's none.
    Path(String),
    /// `font TEXT`: type TEXT in the open settings' font field, scrolled
    /// into view.
    Font(String),
    /// `wait-ready SECS`: wait until no YouTube download is queued or running.
    WaitReady(Duration),
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
    /// `search TEXT`: wait for the index, then search it and the live
    /// sources, logging the hits.
    Search(String),
    /// `show-hit N`: show hit N of the last search now, counted from 1.
    ShowHit(usize),
    /// `type TEXT`: wait for the index, then put TEXT in the dashboard's
    /// search box, as if typed.
    Type(String),
    /// `select N`: pick result N in the dashboard, counted from 1, which
    /// previews it. Web pages are skipped, as on screen.
    Select(usize),
    /// `step N`: click entry N of the plan, counted from 1, sections
    /// included: it becomes the selection.
    Step(usize),
    /// `press KEY`: a dashboard key, as if pressed with the search box empty
    /// (see [`keys::op_named`] for the names). `press ctrl+s` saves
    /// the plan, `ctrl+o` lists saved plans, `ctrl+n` starts a new one.
    Press(String),
    /// `open NAME`: open the saved plan with this title or file name, as if
    /// picked from the open list.
    Open(String),
    /// `rename TEXT`: rename the plan, as if typed into its name.
    Rename(String),
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
                return Err(format!("{word} needs a path or link"));
            }
            "show" => Command::Show(Target::parse(rest, dir)?),
            "preview" => Command::Preview(Target::parse(rest, dir)?),
            "add" => Command::Add(Target::parse(rest, dir)?),
            "slot" if !rest.is_empty() => Command::Slot(rest.to_string()),
            "path" => Command::Path(rest.to_string()),
            "font" => Command::Font(rest.to_string()),
            "slot" => return Err("slot needs what it searches".into()),
            "section" => {
                let usage = || format!("section needs a budget and a title, got {rest:?}");
                let (budget, title) = rest.split_once(' ').ok_or_else(usage)?;
                let budget = crate::length::parse(budget).flatten().ok_or_else(usage)?;
                Command::Section(budget, title.trim().to_string())
            }
            "wait-ready" => {
                let secs = rest
                    .parse()
                    .map_err(|_| format!("wait-ready needs seconds, got {rest:?}"))?;
                Command::WaitReady(Duration::from_secs(secs))
            }
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
            "search" if !rest.is_empty() => Command::Search(rest.to_string()),
            "search" => return Err("search needs text".into()),
            "show-hit" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Command::ShowHit(n - 1),
                _ => return Err(format!("show-hit needs a number from 1, got {rest:?}")),
            },
            "type" if !rest.is_empty() => Command::Type(rest.to_string()),
            "type" => return Err("type needs text".into()),
            "open" | "rename" if rest.is_empty() => return Err(format!("{word} needs a name")),
            "open" => Command::Open(rest.to_string()),
            "rename" => Command::Rename(rest.to_string()),
            "step" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Command::Step(n - 1),
                _ => return Err(format!("step needs a number from 1, got {rest:?}")),
            },
            "select" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Command::Select(n - 1),
                _ => return Err(format!("select needs a number from 1, got {rest:?}")),
            },
            "press" if keys::op_named(rest).is_some() => Command::Press(rest.to_lowercase()),
            "press" => return Err(format!("press needs a key name, got {rest:?}")),
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
                | Command::WaitReady(_)
                | Command::Text(_)
                | Command::Shot(_)
                | Command::Show(_)
                | Command::Preview(_)
                | Command::Add(_)
                | Command::Section(..)
                | Command::Slot(_)
                | Command::Path(_)
                | Command::Font(_)
                | Command::Seek(_)
                | Command::Frame(_)
                | Command::Search(_)
                | Command::ShowHit(_)
                | Command::Type(_)
                | Command::Select(_)
                | Command::Step(_)
                | Command::Press(_)
                | Command::Open(_)
                | Command::Rename(_)
        );
        if !takes_argument && !rest.is_empty() {
            return Err(format!("{word} takes no argument"));
        }
        Ok(command)
    }
}

/// A script command for the dashboard, which only the app's event loop
/// can carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ui {
    Type(String),
    Select(usize),
    Step(usize),
    Path(String),
    Font(String),
    Press(Op),
    Open(String),
    Rename(String),
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
    /// Hands dashboard commands to `ui`. Calls `acted` after each command.
    pub fn run(
        &self,
        engine: &Engine,
        library: &Library,
        downloads: &Arc<Downloads>,
        mut ui: impl FnMut(Ui),
        mut acted: impl FnMut(),
    ) {
        let mut hits = Vec::new();
        for command in &self.commands {
            log(format!("script: {command:?}"));
            match command {
                Command::Quit => return,
                Command::Search(text) => hits = search(engine, library, text),
                Command::ShowHit(n) => match hits.get(*n) {
                    Some(hit) => engine.show_now(hit.resource.clone()),
                    None => log(format!("script: show-hit: only {} hits", hits.len())),
                },
                Command::Type(text) => {
                    if !library.settle(Duration::from_secs(60)) {
                        log("script: type: indexing still running after 60s");
                    }
                    ui(Ui::Type(text.clone()));
                }
                Command::Select(n) => ui(Ui::Select(*n)),
                Command::Step(n) => ui(Ui::Step(*n)),
                Command::Path(text) => ui(Ui::Path(text.clone())),
                Command::Font(text) => ui(Ui::Font(text.clone())),
                Command::Press(key) => ui(Ui::Press(
                    keys::op_named(key).expect("checked by Command::parse"),
                )),
                Command::Open(name) => ui(Ui::Open(name.clone())),
                Command::Rename(name) => ui(Ui::Rename(name.clone())),
                _ => self.perform(command, engine, downloads),
            }
            acted();
        }
    }

    fn perform(&self, command: &Command, engine: &Engine, downloads: &Arc<Downloads>) {
        match command {
            Command::Wait(duration) => std::thread::sleep(*duration),
            Command::WaitReady(timeout) => {
                if !downloads.settle(*timeout) {
                    log(format!("script: wait-ready timed out after {timeout:?}"));
                }
            }
            Command::Hymn => {
                let step = engine.add_resource(None, &hymn());
                if let Err(err) = engine.go_to(step) {
                    log(format!("script: hymn: {err}"));
                }
            }
            Command::Text(body) => engine.show_now(Arc::new(inline("text", "Text", body))),
            Command::Show(target) => engine.show_now(Arc::new(target.resource(downloads))),
            Command::Preview(target) => engine.preview(Arc::new(target.resource(downloads))),
            Command::Add(target) => {
                engine.add_resource(None, &target.resource(downloads));
            }
            Command::Slot(search) => {
                let action = lantern_engine::Action::Live {
                    hint: Some(lantern_engine::Query::new(search.clone())),
                    shown: Vec::new(),
                };
                engine.add(None, "Live search", action);
            }
            Command::Section(budget, title) => {
                let action = lantern_engine::Action::Section {
                    budget: Some(*budget),
                };
                engine.add(None, title.clone(), action);
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
            Command::Quit
            | Command::Search(_)
            | Command::ShowHit(_)
            | Command::Type(_)
            | Command::Select(_)
            | Command::Step(_)
            | Command::Path(_)
            | Command::Font(_)
            | Command::Press(_)
            | Command::Open(_)
            | Command::Rename(_) => {}
        }
    }

    fn shoot(&self, name: &str) {
        let Some(shooter) = &self.shooter else {
            log(format!(
                "script: shot {name} skipped, LANTERN_SHOT is unset"
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

fn search(engine: &Engine, library: &Library, text: &str) -> Vec<Hit> {
    if !library.settle(Duration::from_secs(60)) {
        log("script: search: indexing still running after 60s");
    }
    let mut hits = Vec::new();
    for event in engine.search(Query::new(text)) {
        match event {
            SearchEvent::Hits { hits: batch, .. } => hits.extend(batch),
            SearchEvent::Failed { source, error } => {
                log(format!("script: search: {source} failed: {error}"));
            }
        }
    }
    let titles: Vec<&str> = hits.iter().map(|h| h.resource.title.as_str()).collect();
    log(format!(
        "script: search {text:?} -> {} hits: {}",
        hits.len(),
        titles.join(", ")
    ));
    hits
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
    use crate::config::Config;

    fn run(source: &str) -> Arc<Engine> {
        run_with(Config::default(), source)
    }

    fn run_with(config: Config, source: &str) -> Arc<Engine> {
        let engine = Arc::new(Engine::new());
        let library = Library::start(engine.clone(), config);
        let downloads = Downloads::new("/nonexistent".into());
        Script::parse(source, Path::new("/media"), None)
            .unwrap()
            .run(&engine, &library, &downloads, |_| {}, || {});
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
    fn dashboard_commands_go_to_the_ui() {
        let engine = Arc::new(Engine::new());
        let library = Library::start(engine.clone(), Config::default());
        let downloads = Downloads::new("/nonexistent".into());
        let mut sent = Vec::new();
        let script =
            "type pa\nselect 2\npress PageDown\npress ctrl+s\nrename Sunday 3\nopen Sunday 3";
        Script::parse(script, Path::new(""), None).unwrap().run(
            &engine,
            &library,
            &downloads,
            |ui| sent.push(ui),
            || {},
        );
        assert_eq!(
            sent,
            [
                Ui::Type("pa".into()),
                Ui::Select(1),
                Ui::Press(Op::Next),
                Ui::Press(Op::Save),
                Ui::Rename("Sunday 3".into()),
                Ui::Open("Sunday 3".into()),
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
        assert_eq!(timed.playback, lantern_engine::Playback::Playing);
        assert_eq!(timed.position, Duration::from_millis(1500));

        let engine = run("add a.png\nnext");
        assert_eq!(engine.stage().program.unwrap().resource.title, "a.png");
    }

    #[test]
    fn links_resolve_like_a_pasted_link() {
        let engine = run("add https://youtu.be/jNQXAC9IVRw\nnext");
        let resource = engine.stage().program.unwrap().resource;
        assert_eq!(resource.kind, Kind::Video);
        assert_eq!(resource.meta["youtube_id"], "jNQXAC9IVRw");
        assert_eq!(
            resource.location,
            Location::Url("https://www.youtube.com/watch?v=jNQXAC9IVRw".into())
        );
    }

    #[test]
    fn search_then_show_hit_puts_a_bible_passage_on_stage() {
        let bible = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../sources/bible/fixtures/sample.tsv"
        );
        let config = format!("[[bibles]]\nlang = \"es\"\npath = \"{bible}\"");
        let config = Config::parse(&config, Path::new("/"), None).unwrap();
        let engine = run_with(config, "search juan 3:16-17\nshow-hit 1");
        let program = engine.stage().program.unwrap();
        assert_eq!(program.resource.title, "Juan 3:16-17");
        assert!(program.text().unwrap().starts_with("Porque de tal manera"));
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
        assert_eq!(err("search"), "line 1: search needs text");
        assert_eq!(err("type"), "line 1: type needs text");
        assert_eq!(err("open"), "line 1: open needs a name");
        assert_eq!(
            err("select 0"),
            r#"line 1: select needs a number from 1, got "0""#
        );
        assert_eq!(
            err("press f13"),
            r#"line 1: press needs a key name, got "f13""#
        );
        assert_eq!(
            err("show-hit x"),
            r#"line 1: show-hit needs a number from 1, got "x""#
        );
        assert_eq!(err("show"), "line 1: show needs a path or link");
        assert_eq!(err("show https://"), r#"line 1: not a link: "https://""#);
        assert_eq!(
            err("wait-ready soon"),
            r#"line 1: wait-ready needs seconds, got "soon""#
        );
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
