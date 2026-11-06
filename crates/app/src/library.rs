//! The sources the config names, registered and indexed on a worker thread
//! so the windows open at once.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use chorquest_engine::{Engine, SourceId};
use chorquest_source_bible::{Bible, BibleSource, Translation};
use chorquest_source_link::LinkSource;
use chorquest_source_local::{Folder, LocalSource};

use crate::config::{BibleEntry, Config};
use crate::log;

/// How one source fared: its indexed item count, or why it has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indexed {
    pub source: String,
    pub result: Result<usize, String>,
}

/// The index's progress. `None` until indexing finishes.
#[derive(Debug, Default)]
pub struct Library {
    reports: Mutex<Option<Vec<Indexed>>>,
    done: Condvar,
}

impl Library {
    /// Registers the link source now, and the configured sources and their
    /// index on a worker thread.
    pub fn start(engine: Arc<Engine>, config: Config) -> Arc<Library> {
        if let Err(err) = engine.add_source(LinkSource::new(id("link"))) {
            log(format!("link source: {err}"));
        }
        let library = Arc::new(Library::default());
        let worker = library.clone();
        std::thread::spawn(move || {
            let mut reports = register(&engine, config);
            reports.extend(engine.reindex().into_iter().map(|report| Indexed {
                source: report.source.to_string(),
                result: report.result.map_err(|err| err.to_string()),
            }));
            for line in summary(Some(&reports)) {
                log(line);
            }
            *worker.lock() = Some(reports);
            worker.done.notify_all();
        });
        library
    }

    fn lock(&self) -> MutexGuard<'_, Option<Vec<Indexed>>> {
        self.reports.lock().unwrap_or_else(|e| e.into_inner())
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

    /// The dashboard's lines about the index.
    pub fn lines(&self) -> Vec<String> {
        summary(self.lock().as_deref())
    }
}

/// Registers one local source for every folder and one source per Bible.
/// Returns a report for each Bible that couldn't be loaded or registered.
fn register(engine: &Engine, config: Config) -> Vec<Indexed> {
    let local = config.folders.into_iter().fold(
        LocalSource::new(id("local"), "Local folders"),
        |local, entry| {
            let folder = Folder::new(entry.path);
            local.with_folder(match entry.label {
                Some(label) => folder.labeled(label),
                None => folder,
            })
        },
    );
    let mut failed = Vec::new();
    if let Err(err) = engine.add_source(local) {
        failed.push(Indexed {
            source: "local".into(),
            result: Err(err.to_string()),
        });
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
                    .add_source(BibleSource::new(id, bible))
                    .map_err(|err| err.to_string())
            });
        if let Err(error) = result {
            failed.push(Indexed {
                source,
                result: Err(error),
            });
        }
    }
    failed
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

    use chorquest_engine::{Query, SearchEvent};

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
        let lines = library.lines();
        assert_eq!(lines[0], "Index: 12 items (local 4, bible-rv1909 8)");
        assert!(
            lines[1].starts_with("  bible-kjv failed: ") && lines[1].contains("kjv.json"),
            "{lines:?}"
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
        let (_, library) = started("");
        assert_eq!(library.lines(), ["Index: 0 items (local 0)"]);
    }
}
