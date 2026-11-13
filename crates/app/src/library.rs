//! The sources the config names, registered and indexed on a worker thread
//! so the windows open at once, and swapped for new ones when the settings
//! change.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use lantern_engine::{Engine, SourceId};
use lantern_source_bible::{Bible, BibleSource, Translation};
use lantern_source_brave::BraveImages;
use lantern_source_link::LinkSource;
use lantern_source_local::{Folder, LocalSource};
use lantern_source_youtube::YoutubeSource;

use crate::config::{self, BibleEntry, Config, FolderEntry};
use crate::{log, watch};

/// How one source fared: its indexed item count, or why it has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indexed {
    pub source: String,
    pub result: Result<usize, String>,
}

/// The configured sources and the index's progress.
pub struct Library {
    engine: Arc<Engine>,
    /// `None` while indexing.
    reports: Mutex<Option<Vec<Indexed>>>,
    done: Condvar,
    /// The sources the config registered, to take out on the next apply.
    configured: Mutex<Vec<SourceId>>,
    /// One apply at a time.
    applying: Mutex<()>,
    /// Watches the folders while it lives, so new files show up.
    watch: Mutex<Option<watch::Watch>>,
    /// Counts the scans the watch made, so a search on screen can follow.
    changes: Arc<AtomicU64>,
    /// Counts applies, so only the latest one says indexing is done.
    generation: AtomicU64,
}

impl Library {
    /// Registers the sources the config doesn't change (links, YouTube) now,
    /// and the configured ones and their index on a worker thread.
    pub fn start(engine: Arc<Engine>, config: Config) -> Arc<Library> {
        let fixed = [
            engine.add_source(LinkSource::new(id("link"))),
            engine.add_source(YoutubeSource::new(id("youtube"))),
        ];
        for err in fixed.into_iter().filter_map(Result::err) {
            log(format!("source: {err}"));
        }
        let library = Arc::new(Library {
            engine,
            reports: Mutex::new(None),
            done: Condvar::new(),
            configured: Mutex::new(Vec::new()),
            applying: Mutex::new(()),
            watch: Mutex::new(None),
            changes: Arc::default(),
            generation: AtomicU64::new(0),
        });
        library.apply(config);
        library
    }

    /// Swaps the configured sources (folders, Bibles, image search) for
    /// `config`'s and reindexes, on a worker thread. Searches keep working
    /// meanwhile; the dashboard says it's indexing until done.
    pub fn apply(self: &Arc<Self>, config: Config) {
        *self.lock() = None;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let library = Arc::clone(self);
        std::thread::spawn(move || {
            let _one_at_a_time = library.applying.lock().unwrap_or_else(|e| e.into_inner());
            let engine = &library.engine;
            let old =
                std::mem::take(&mut *library.configured.lock().unwrap_or_else(|e| e.into_inner()));
            for source in old {
                let _ = engine.searcher().unregister(&source);
            }
            let folders: Vec<_> = config
                .folders
                .iter()
                .map(|entry| (entry.path.clone(), folder_source(entry)))
                .collect();
            let (registered, mut reports) = register(engine, config);
            *library.configured.lock().unwrap_or_else(|e| e.into_inner()) = registered;
            reports.extend(engine.reindex().into_iter().map(|report| Indexed {
                source: report.source.to_string(),
                result: report.result.map_err(|err| err.to_string()),
            }));
            for line in summary(Some(&reports)) {
                log(line);
            }
            // Watch the folders from now on: a change scans its source again.
            let watching = watch::start(folders, Arc::clone(engine), Arc::clone(&library.changes));
            *library.watch.lock().unwrap_or_else(|e| e.into_inner()) = watching;
            if library.generation.load(Ordering::SeqCst) == generation {
                *library.lock() = Some(reports);
                library.done.notify_all();
            }
        });
    }

    fn lock(&self) -> MutexGuard<'_, Option<Vec<Indexed>>> {
        self.reports.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// How many times a watched folder changed the index so far.
    pub fn changes(&self) -> u64 {
        self.changes.load(Ordering::SeqCst)
    }

    pub fn indexing(&self) -> bool {
        self.lock().is_none()
    }

    /// Blocks until indexing finishes or `timeout` passes. True if it finished.
    pub fn settle(&self, timeout: Duration) -> bool {
        let reports = self.lock();
        let (reports, _) = self
            .done
            .wait_timeout_while(reports, timeout, |r| r.is_none())
            .unwrap_or_else(|e| e.into_inner());
        reports.is_some()
    }

    /// What the dashboard should say about the index: that it's still
    /// indexing, or which sources failed. Nothing once all is well.
    pub fn problems(&self) -> Vec<String> {
        problems(self.lock().as_deref())
    }
}

fn problems(reports: Option<&[Indexed]>) -> Vec<String> {
    match reports {
        None => vec!["Indexing your folders and Bibles…".into()],
        Some(reports) => summary(Some(reports))
            .into_iter()
            .skip(1)
            .map(|line| line.trim().to_string())
            .collect(),
    }
}

/// Registers the folders (one local source for those without a bang, and
/// one per bang for the rest), one source per Bible, and
/// image search with the config's key. Returns the ids registered, and a
/// report for each source that couldn't be loaded or registered.
fn register(engine: &Engine, config: Config) -> (Vec<SourceId>, Vec<Indexed>) {
    let mut registered = Vec::new();
    let mut failed = Vec::new();
    let mut add = |source: String, result: Result<SourceId, String>| match result {
        Ok(id) => registered.push(id),
        Err(error) => failed.push(Indexed {
            source,
            result: Err(error),
        }),
    };
    add(
        "images".into(),
        engine
            .add_source(BraveImages::new(id("images"), config.brave_api_key))
            .map(|()| id("images"))
            .map_err(|err| err.to_string()),
    );
    let mut bad = Vec::new();
    let locals = folder_sources(config.folders, &mut |source, error| {
        bad.push((source, error))
    });
    for (source, error) in bad {
        add(source, Err(error));
    }
    for (source, local) in locals {
        let result = engine
            .add_source(local)
            .map(|()| id(&source))
            .map_err(|err| err.to_string());
        add(source, result);
    }
    for entry in config.bibles {
        let name = bible_name(&entry);
        let source = format!("bible-{}", slug(&name));
        let result = SourceId::new(&source)
            .map_err(|err| err.to_string())
            .and_then(|id| {
                let translation = Translation::new(name.clone(), name, entry.lang.into());
                let bible = Bible::load(translation, &entry.path)
                    .map_err(|err| format!("{}: {err}", entry.path.display()))?;
                engine
                    .add_source(BibleSource::new(id.clone(), bible))
                    .map(|()| id)
                    .map_err(|err| err.to_string())
            });
        add(source, result);
    }
    (registered, failed)
}

/// The local sources for `folders`, with their source ids: `local` for
/// the folders without a bang (left out when every folder has one), then `local-h` for
/// the folders with `!h`, in the order their bang first appears. A bang
/// that can't be one is reported with `bad` and the folder goes in `local`.
/// The source a folder belongs to: `local-h` for a folder with `!h`,
/// `local` without a bang or with one that can't be.
fn folder_source(entry: &FolderEntry) -> SourceId {
    match entry.bang.as_deref().map(config::folder_bang) {
        Some(Ok(bang)) => id(&format!("local-{bang}")),
        _ => id("local"),
    }
}

fn folder_sources(
    folders: Vec<FolderEntry>,
    bad: &mut dyn FnMut(String, String),
) -> Vec<(String, LocalSource)> {
    let mut groups: Vec<(Option<String>, Vec<FolderEntry>)> = vec![(None, Vec::new())];
    for entry in folders {
        let bang = entry.bang.as_deref().and_then(|text| {
            config::folder_bang(text)
                .map_err(|err| bad(format!("folder {}", entry.path.display()), err))
                .ok()
        });
        match groups.iter_mut().find(|(b, _)| *b == bang) {
            Some((_, entries)) => entries.push(entry),
            None => groups.push((bang, vec![entry])),
        }
    }
    let every_one_banged = groups[0].1.is_empty() && groups.len() > 1;
    groups
        .into_iter()
        .skip(usize::from(every_one_banged))
        .map(|(bang, entries)| {
            let (source, local) = match &bang {
                None => (
                    "local".to_string(),
                    LocalSource::new(id("local"), "Local folders"),
                ),
                Some(bang) => {
                    let names: Vec<String> = entries.iter().map(folder_name).collect();
                    let source = format!("local-{bang}");
                    let local = LocalSource::new(id(&source), names.join(", ")).with_bang(bang);
                    (source, local)
                }
            };
            let local = entries.into_iter().fold(local, |local, entry| {
                let folder = Folder::new(entry.path);
                local.with_folder(match entry.label {
                    Some(label) => folder.labeled(label),
                    None => folder,
                })
            });
            (source, local)
        })
        .collect()
}

/// A folder's label, or else its own name.
fn folder_name(entry: &FolderEntry) -> String {
    entry.label.clone().unwrap_or_else(|| {
        entry.path.file_name().map_or_else(
            || entry.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    })
}

fn bible_name(entry: &BibleEntry) -> String {
    entry.name.clone().unwrap_or_else(|| {
        entry
            .path
            .file_stem()
            .map_or("bible".into(), |s| s.to_string_lossy().to_uppercase())
    })
}

fn slug(name: &str) -> String {
    name.to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn id(id: &str) -> SourceId {
    SourceId::new(id).expect("a valid source id")
}

/// "Index: 11 items (local 3, bible-rv1909 8)", then a line per failed source.
fn summary(reports: Option<&[Indexed]>) -> Vec<String> {
    let Some(reports) = reports else {
        return vec!["Index: indexing…".into()];
    };
    let counts: Vec<(&str, usize)> = reports
        .iter()
        .filter_map(|r| r.result.as_ref().ok().map(|n| (r.source.as_str(), *n)))
        .collect();
    let total: usize = counts.iter().map(|(_, n)| n).sum();
    let each: Vec<String> = counts.iter().map(|(s, n)| format!("{s} {n}")).collect();
    let mut lines = vec![format!("Index: {total} items ({})", each.join(", "))];
    lines.extend(reports.iter().filter_map(|r| {
        r.result
            .as_ref()
            .err()
            .map(|err| format!("  {} failed: {err}", r.source))
    }));
    lines
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use lantern_engine::{Query, SearchEvent};

    use super::*;
    use crate::config::Config;

    fn repo() -> &'static Path {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
    }

    fn started(config: &str) -> (Arc<Engine>, Arc<Library>) {
        let config = Config::parse(config, repo(), None).unwrap();
        let engine = Arc::new(Engine::new());
        let library = Library::start(engine.clone(), config);
        assert!(library.settle(Duration::from_secs(30)), "indexing finished");
        (engine, library)
    }

    fn bang_names(engine: &Engine) -> Vec<(String, Vec<String>)> {
        engine
            .searcher()
            .bangs()
            .into_iter()
            .map(|b| (b.bang, b.sources))
            .collect()
    }

    #[test]
    fn the_taken_bangs_are_the_ones_lantern_answers_on_its_own() {
        let (engine, _library) = started(
            r#"
            [[folders]]
            path = "scripts/e2e/fixtures"
            [[bibles]]
            lang = "es"
            path = "sources/bible/fixtures/sample.tsv"
            "#,
        );
        let own: Vec<String> = bang_names(&engine).into_iter().map(|(b, _)| b).collect();
        assert_eq!(own, config::TAKEN_BANGS);
    }

    #[test]
    fn a_folder_with_a_bang_answers_it_and_still_answers_f() {
        let (engine, library) = started(
            r#"
            [[folders]]
            path = "scripts/e2e/fixtures"
            label = "Fixtures"
            bang = "!FX"
            [[folders]]
            path = "sources/bible/fixtures"
            bang = "b"
            "#,
        );
        let bangs = bang_names(&engine);
        let sources = |bang: &str| {
            bangs
                .iter()
                .find(|(b, _)| b == bang)
                .map(|(_, s)| s.clone())
        };
        assert_eq!(sources("fx"), Some(vec!["Fixtures".to_string()]));
        assert_eq!(
            sources("f"),
            Some(vec!["Local folders".to_string(), "Fixtures".to_string()]),
            "the folder with a bad bang is in Local folders"
        );
        let only = |text: &str| -> Vec<String> {
            let query = engine.searcher().parse(text).query;
            let mut titles: Vec<String> = engine
                .searcher()
                .search_index(&query)
                .into_iter()
                .map(|hit| hit.resource.title.clone())
                .collect();
            titles.sort();
            titles
        };
        assert_eq!(only("!fx image"), ["image"]);
        assert_eq!(only("!f image"), ["image"], "!f still finds it");
        assert_eq!(sources("b"), None, "the bad bang wasn't given");
        let problems = library.problems();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("!b is taken"), "{problems:?}");
    }

    /// Waits up to five seconds for `found` to hold.
    fn eventually(found: impl Fn() -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if found() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        found()
    }

    #[test]
    fn a_file_added_to_a_watched_folder_shows_up_and_leaves() {
        let plain = tempfile::tempdir().unwrap();
        let hymns = tempfile::tempdir().unwrap();
        let config = format!(
            "[[folders]]\npath = {:?}\n[[folders]]\npath = {:?}\nbang = \"h\"\n",
            plain.path(),
            hymns.path()
        );
        let (engine, _library) = started(&config);
        let titles = |text: &str| -> Vec<String> {
            let query = engine.searcher().parse(text).query;
            engine
                .searcher()
                .search_index(&query)
                .into_iter()
                .map(|hit| hit.resource.title.clone())
                .collect()
        };
        assert!(titles("cordero").is_empty());

        let image = repo().join("scripts/e2e/fixtures/image.png");
        std::fs::copy(&image, hymns.path().join("cordero.png")).unwrap();
        assert!(
            eventually(|| titles("!h cordero") == ["cordero"]),
            "found under !h"
        );
        std::fs::copy(&image, plain.path().join("cordero de dios.png")).unwrap();
        assert!(
            eventually(|| titles("!f cordero").len() == 2),
            "{:?}",
            titles("!f cordero")
        );

        std::fs::remove_file(hymns.path().join("cordero.png")).unwrap();
        assert!(eventually(|| titles("!h cordero").is_empty()), "gone");
        assert_eq!(titles("!f cordero"), ["cordero de dios"]);
    }

    #[test]
    fn indexes_configured_folders_and_bibles_and_reports_failures() {
        let (engine, library) = started(
            r#"
            [[folders]]
            path = "scripts/e2e/fixtures"
            [[bibles]]
            lang = "es"
            path = "sources/bible/fixtures/sample.tsv"
            name = "RV1909"
            [[bibles]]
            lang = "en"
            path = "missing/kjv.json"
            "#,
        );
        assert_eq!(engine.searcher().index().len(), 12, "4 files and 8 verses");
        let problems = library.problems();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].starts_with("bible-kjv failed: ") && problems[0].contains("kjv.json"),
            "{problems:?}"
        );

        let titles: Vec<String> = engine
            .search(Query::new("jn 3:16"))
            .flat_map(|event| match event {
                SearchEvent::Hits { hits, .. } => hits,
                SearchEvent::Failed { .. } => Vec::new(),
            })
            .map(|hit| hit.resource.title.clone())
            .collect();
        assert!(titles.contains(&"Juan 3:16".to_string()), "{titles:?}");
    }

    #[test]
    fn says_indexing_until_done() {
        assert_eq!(summary(None), ["Index: indexing…"]);
        assert_eq!(problems(None), ["Indexing your folders and Bibles…"]);
        let (_, library) = started("");
        assert!(library.problems().is_empty());
    }

    #[test]
    fn applying_a_new_config_swaps_the_sources() {
        let (engine, library) = started(
            r#"
            [[folders]]
            path = "scripts/e2e/fixtures"
            "#,
        );
        assert_eq!(engine.searcher().index().len(), 4);
        let bible_only = Config::parse(
            r#"
            [[bibles]]
            lang = "es"
            path = "sources/bible/fixtures/sample.tsv"
            name = "RV1909"
            "#,
            repo(),
            None,
        )
        .unwrap();
        library.apply(bible_only);
        assert!(library.settle(Duration::from_secs(30)));
        assert_eq!(engine.searcher().index().len(), 8, "the verses, no files");
        let ids: Vec<String> = engine
            .searcher()
            .sources()
            .iter()
            .map(|s| s.id().to_string())
            .collect();
        assert_eq!(
            ids.iter().filter(|id| *id == "images").count(),
            1,
            "{ids:?}"
        );
    }
}
