//! Try the engine from a terminal.
//!
//! ```sh
//! cargo run --example demo -- ~/Music ~/Videos ~/Pictures
//! cargo run --example demo -- --bible es:sources/bible/fixtures/sample.tsv ~/Music
//! ```
//!
//! `--bible LANG:FILE` loads a translation (`.json` or `.tsv`, see the
//! `lantern-source-bible` docs); `LANG` is `es` or `en` and sets the book names.
//!
//! A background thread plays the part of the stage window and prints every
//! `[stage]` event; the prompt is the dashboard. `:frames N` and `:end` send
//! the reports a real stage window would. Type `:help` for commands.

use std::io::{self, BufRead, Write};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use lantern_engine::{
    Action, Engine, Hit, MediaReport, Origin, Query, SearchEvent, SourceId, StageEvent,
};
use lantern_source_bible::{Bible, BibleSource, Language, Translation};
use lantern_source_link::LinkSource;
use lantern_source_local::{Folder, LocalSource};

const HELP: &str = "\
  <text>        search (index + pasted links + Bible references like `jn 3:16-17`)
  :show N       put result N on stage now
  :pre N        preview result N      :take   put the preview on stage
  :add N        append result N to the plan
  :live LABEL   append a live slot    :blank  append a blank step
  :plan         print the plan        :log    print what was shown
  :n / :p       next / previous (frame, then step)
  :resume       re-show the current step
  :x            toggle blackout       :play / :pause
  :seek SECS    jump the video        :frames N  report N pages for the program
  :end          report that the program's video ended
  :save FILE    save the plan         :open FILE
  :q            quit";

fn main() {
    let engine = Arc::new(Engine::new());

    let mut local = LocalSource::new(id("local"), "Local folders");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--bible" {
            let spec = args
                .next()
                .expect("--bible needs LANG:FILE, e.g. es:rv1909.json");
            add_bible(&engine, &spec);
        } else {
            local = local.with_folder(Folder::new(arg));
        }
    }
    engine.add_source(local).expect("unique id");
    engine
        .add_source(LinkSource::new(id("link")))
        .expect("unique id");

    for report in engine.reindex() {
        match report.result {
            Ok(count) => println!("indexed {count} items from {}", report.source),
            Err(error) => println!("could not index {}: {error}", report.source),
        }
    }

    // The "stage window".
    let feed = engine.subscribe();
    thread::spawn(move || {
        for event in feed.events {
            match event {
                StageEvent::Preview { .. } => {} // the audience never sees previews
                StageEvent::Program { scene: Some(scene) } => {
                    let text = scene.text().map(|t| format!(": {t:?}")).unwrap_or_default();
                    println!(
                        "[stage] showing {} ({}){text}",
                        scene.resource.title, scene.resource.kind
                    );
                }
                other => println!("[stage] {other:?}"),
            }
        }
    });

    println!("{HELP}");
    let mut results: Vec<Hit> = Vec::new();
    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush().ok();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let line = line.trim();
        let (command, arg) = line.split_once(' ').unwrap_or((line, ""));
        let pick = |results: &[Hit]| {
            arg.parse::<usize>()
                .ok()
                .and_then(|n| results.get(n))
                .cloned()
        };

        match command {
            "" => {}
            ":q" => break,
            ":help" => println!("{HELP}"),
            ":show" => match pick(&results) {
                Some(hit) => engine.show_now(hit.resource),
                None => println!("no such result"),
            },
            ":pre" => match pick(&results) {
                Some(hit) => {
                    println!("previewing {}", hit.resource.title);
                    engine.preview(hit.resource);
                }
                None => println!("no such result"),
            },
            ":take" => {
                if !engine.take() {
                    println!("nothing in preview");
                }
            }
            ":add" => match pick(&results) {
                Some(hit) => {
                    engine.edit_plan(|plan| plan.show((*hit.resource).clone()));
                }
                None => println!("no such result"),
            },
            ":live" => {
                engine.edit_plan(|plan| plan.live(arg, None));
            }
            ":blank" => {
                engine.edit_plan(|plan| plan.push("Blank", Action::Blank));
            }
            ":plan" => print_plan(&engine),
            ":log" => {
                for entry in engine.log() {
                    let how = if entry.planned { "planned" } else { "live" };
                    println!("  {how:8} {}", entry.resource.title);
                }
            }
            ":n" => println!("{:?}", engine.next()),
            ":p" => println!("{:?}", engine.prev()),
            ":resume" => println!("{:?}", engine.resume()),
            ":x" => {
                engine.toggle_blackout();
            }
            ":play" => {
                if !engine.play() {
                    println!("nothing to play");
                }
            }
            ":pause" => {
                if !engine.pause() {
                    println!("nothing to pause");
                }
            }
            ":seek" => match arg
                .parse::<f64>()
                .ok()
                .and_then(|s| Duration::try_from_secs_f64(s).ok())
            {
                Some(position) => {
                    if !engine.seek(position) {
                        println!("nothing to seek");
                    }
                }
                None => println!("usage: :seek SECS"),
            },
            ":frames" => match arg.parse::<NonZeroUsize>() {
                Ok(count) => report_on_program(&engine, MediaReport::Frames(count)),
                Err(_) => println!("usage: :frames N (at least 1)"),
            },
            ":end" => report_on_program(&engine, MediaReport::Ended),
            ":save" => report(engine.save_plan(arg)),
            ":open" => report(engine.open_plan(arg)),
            _ if command.starts_with(':') => println!("unknown command; :help lists them"),
            _ => results = search(&engine, line),
        }
    }
}

fn search(engine: &Engine, text: &str) -> Vec<Hit> {
    let mut all = Vec::new();
    // Blocking iteration is fine here; a UI would `poll()` instead.
    for event in engine.search(Query::new(text).with_limit(15)) {
        match event {
            SearchEvent::Hits { origin, hits } => {
                let from = match origin {
                    Origin::Index => "index".to_owned(),
                    Origin::Source(id) => id.to_string(),
                };
                for hit in hits {
                    let r = &hit.resource;
                    let subtitle = r.subtitle.as_deref().unwrap_or("");
                    println!(
                        "  [{}] {} ({}, {from}) {subtitle}",
                        all.len(),
                        r.title,
                        r.kind
                    );
                    all.push(hit);
                }
            }
            SearchEvent::Failed { source, error } => println!("  {source} failed: {error}"),
        }
    }
    if all.is_empty() {
        println!("  nothing found");
    }
    all
}

fn print_plan(engine: &Engine) {
    let plan = engine.plan();
    let current = engine.current_step().map(|s| s.id);
    println!("{}", plan.title);
    for step in plan.steps() {
        let marker = if Some(step.id) == current { ">" } else { " " };
        let what = match &step.action {
            Action::Show { resource } => format!("show {}", resource.kind),
            Action::Live { shown, .. } => format!("live ({} shown)", shown.len()),
            Action::Blank => "blank".into(),
            Action::Note => "note".into(),
        };
        println!(" {marker} {} {:30} {what}", step.id, step.label);
    }
}

/// Reports on behalf of a stage window, about the scene on program now.
fn report_on_program(engine: &Engine, report: MediaReport) {
    match engine.stage().program {
        Some(scene) => {
            if !engine.report(scene.id, report) {
                println!("the program ignored that report");
            }
        }
        None => println!("nothing on stage"),
    }
}

fn report<E: std::fmt::Display>(result: Result<(), E>) {
    match result {
        Ok(()) => println!("ok"),
        Err(error) => println!("error: {error}"),
    }
}

/// `es:/path/rv1909.json` → a source `bible-rv1909` with Spanish book names.
fn add_bible(engine: &Engine, spec: &str) {
    let (language, path) = match spec.split_once(':') {
        Some(("es", path)) => (Language::Spanish, path),
        Some(("en", path)) => (Language::English, path),
        _ => (Language::Spanish, spec),
    };
    let stem = Path::new(path)
        .file_stem()
        .map_or("bible".into(), |s| s.to_string_lossy().into_owned());
    let slug: String = stem
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let translation = Translation::new(stem.to_uppercase(), stem.clone(), language);
    match Bible::load(translation, path) {
        Ok(bible) => {
            println!("loaded {} verses from {path}", bible.verse_count());
            let source = BibleSource::new(id(&format!("bible-{slug}")), bible);
            engine.add_source(source).expect("unique id");
        }
        Err(error) => println!("could not load {path}: {error}"),
    }
}

fn id(id: &str) -> SourceId {
    SourceId::new(id).expect("valid id")
}
