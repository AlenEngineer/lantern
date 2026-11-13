use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread;

use lantern_resource::{Hit, Query, Resource, Source, SourceError, SourceId};

use crate::Index;
use crate::bang::{self, Bang, Parsed, Scope};

/// Errors from managing sources.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// Two sources can't share an id.
    #[error("a source with id {0} is already registered")]
    DuplicateSource(SourceId),
    /// No source has that id.
    #[error("no source with id {0}")]
    UnknownSource(SourceId),
    /// The source itself failed.
    #[error("source {id} failed: {error}")]
    Source {
        /// Which source.
        id: SourceId,
        /// What went wrong.
        #[source]
        error: SourceError,
    },
}

/// Outcome of re-scanning one indexed source.
#[derive(Debug)]
pub struct ReindexReport {
    /// Which source.
    pub source: SourceId,
    /// How many resources it now has in the index. On error, whatever it had
    /// before is kept, so a disconnected drive doesn't empty the library.
    pub result: Result<usize, SourceError>,
}

/// Where a batch of results came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The in-memory index (all indexed sources together).
    Index,
    /// A live source.
    Source(SourceId),
}

/// Something that happened while a search ran.
#[derive(Debug)]
pub enum SearchEvent {
    /// Results from one origin. Scores are only comparable within one batch.
    Hits {
        /// Who answered.
        origin: Origin,
        /// Best first.
        hits: Vec<Hit>,
    },
    /// A live source failed. Other origins are unaffected.
    Failed {
        /// Which source.
        source: SourceId,
        /// Human-readable reason.
        error: String,
    },
}

/// Non-blocking poll result of a [`SearchStream`].
#[derive(Debug)]
pub enum Polled {
    /// Something arrived.
    Event(SearchEvent),
    /// Nothing yet; some sources are still working.
    Pending,
    /// Every origin has answered.
    Finished,
}

/// The results of one search, arriving over time.
///
/// The index batch is always first and is available immediately. Live sources
/// follow in whatever order they finish. Iterating blocks until everything has
/// answered; a UI should call [`poll`](Self::poll) from its event loop instead.
///
/// Each stream has a [`ticket`](Self::ticket). When the operator types
/// another letter, start a new search and drop the old stream: its late
/// results are discarded.
pub struct SearchStream {
    ticket: u64,
    rx: Receiver<SearchEvent>,
}

impl SearchStream {
    /// Increasing number identifying this search.
    pub fn ticket(&self) -> u64 {
        self.ticket
    }

    /// Checks for a new event without blocking.
    pub fn poll(&self) -> Polled {
        match self.rx.try_recv() {
            Ok(event) => Polled::Event(event),
            Err(TryRecvError::Empty) => Polled::Pending,
            Err(TryRecvError::Disconnected) => Polled::Finished,
        }
    }
}

impl Iterator for SearchStream {
    type Item = SearchEvent;
    fn next(&mut self) -> Option<SearchEvent> {
        self.rx.recv().ok()
    }
}

/// Registry of sources plus the shared index. Cheap to share behind an `Arc`.
#[derive(Default)]
pub struct Searcher {
    sources: RwLock<Vec<Arc<dyn Source>>>,
    index: RwLock<Index>,
    tickets: AtomicU64,
}

impl Searcher {
    /// No sources, empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a source. It is not scanned until [`reindex`](Self::reindex) or
    /// [`reindex_all`](Self::reindex_all) is called.
    pub fn register(&self, source: Arc<dyn Source>) -> Result<(), SearchError> {
        let mut sources = write(&self.sources);
        if sources.iter().any(|s| s.id() == source.id()) {
            return Err(SearchError::DuplicateSource(source.id().clone()));
        }
        sources.push(source);
        Ok(())
    }

    /// Removes a source and everything it had indexed.
    pub fn unregister(&self, id: &SourceId) -> Result<(), SearchError> {
        let mut sources = write(&self.sources);
        let before = sources.len();
        sources.retain(|s| s.id() != id);
        if sources.len() == before {
            return Err(SearchError::UnknownSource(id.clone()));
        }
        write(&self.index).remove_source(id);
        Ok(())
    }

    /// All registered sources, in registration order.
    pub fn sources(&self) -> Vec<Arc<dyn Source>> {
        read(&self.sources).clone()
    }

    /// Read access to the index. Don't hold it for long: reindexing waits on it.
    pub fn index(&self) -> RwLockReadGuard<'_, Index> {
        read(&self.index)
    }

    /// Re-scans one source. Returns how many resources it now has indexed.
    pub fn reindex(&self, id: &SourceId) -> Result<usize, SearchError> {
        let source = self
            .sources()
            .into_iter()
            .find(|s| s.id() == id)
            .ok_or_else(|| SearchError::UnknownSource(id.clone()))?;
        if !source.capabilities().scan {
            return Ok(0);
        }
        // Scan without holding the lock, so searches keep working meanwhile.
        let items = source.scan().map_err(|error| SearchError::Source {
            id: id.clone(),
            error,
        })?;
        Ok(write(&self.index).replace_source(id, items))
    }

    /// Re-scans every indexed source, in parallel.
    pub fn reindex_all(&self) -> Vec<ReindexReport> {
        let sources: Vec<_> = self
            .sources()
            .into_iter()
            .filter(|s| s.capabilities().scan)
            .collect();

        let scanned: Vec<(SourceId, Result<Vec<Resource>, SourceError>)> = thread::scope(|scope| {
            let handles: Vec<_> = sources
                .iter()
                .map(|source| scope.spawn(move || source.scan()))
                .collect();
            sources
                .iter()
                .zip(handles)
                .map(|(source, handle)| {
                    let result = handle
                        .join()
                        .unwrap_or_else(|_| Err(SourceError::Unavailable("scan panicked".into())));
                    (source.id().clone(), result)
                })
                .collect()
        });

        let mut index = write(&self.index);
        scanned
            .into_iter()
            .map(|(source, result)| ReindexReport {
                result: result.map(|items| index.replace_source(&source, items)),
                source,
            })
            .collect()
    }

    /// Searches only the in-memory index. Instant; fine to call on every keystroke.
    pub fn search_index(&self, query: &Query) -> Vec<Hit> {
        read(&self.index).search(query)
    }

    /// Searches the index and every live source that `query` allows. A
    /// source [on request](lantern_resource::Capabilities::on_request) is
    /// only asked when the query names it in its source filter.
    ///
    /// Each live source runs on its own thread, so a slow network never delays
    /// local results. Live sources are usually remote: debounce keystrokes
    /// before calling this, and use [`search_index`](Self::search_index) while typing.
    pub fn search(&self, query: Query) -> SearchStream {
        let ticket = self.tickets.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::channel();

        // Can't fail: we still hold `rx`.
        let _ = tx.send(SearchEvent::Hits {
            origin: Origin::Index,
            hits: self.search_index(&query),
        });

        let query = Arc::new(query);
        let live = self.sources().into_iter().filter(|s| {
            let caps = s.capabilities();
            caps.search
                && query.wants_source(s.id())
                && (!caps.on_request || query.sources.contains(s.id()))
        });
        for source in live {
            let tx = tx.clone();
            let query = Arc::clone(&query);
            thread::spawn(move || {
                let id = source.id().clone();
                let event = match source.search(&query) {
                    Ok(items) => SearchEvent::Hits {
                        origin: Origin::Source(id),
                        hits: rank(items, &query),
                    },
                    Err(error) => SearchEvent::Failed {
                        source: id,
                        error: error.to_string(),
                    },
                };
                // The receiver may be gone if a newer search replaced this one.
                let _ = tx.send(event);
            });
        }

        SearchStream { ticket, rx }
    }

    /// Reads typed text for a leading bang: `!b juan 3` is a query for
    /// `juan 3` that only the sources whose bang is `b` answer. Bangs are
    /// matched without regard to case.
    pub fn parse(&self, text: &str) -> Parsed {
        let Some((bang, rest)) = bang::split(text) else {
            return Parsed {
                query: Query::new(text.trim()),
                scope: Scope::Everything,
            };
        };
        let bang = bang.to_lowercase();
        let query = self
            .sources()
            .iter()
            .filter(|s| s.bangs().iter().any(|b| b.eq_ignore_ascii_case(&bang)))
            .fold(Query::new(rest), |query, s| {
                query.with_source(s.id().clone())
            });
        let scope = match self.bangs().into_iter().find(|b| b.bang == bang) {
            Some(found) => Scope::Bang(found),
            None => Scope::Unknown(bang),
        };
        Parsed { query, scope }
    }

    /// Every bang the registered sources answer, in alphabetical order.
    pub fn bangs(&self) -> Vec<Bang> {
        let mut bangs: Vec<Bang> = Vec::new();
        for source in self.sources() {
            let on_request = source.capabilities().on_request;
            for name in source.bangs() {
                let name = name.to_lowercase();
                match bangs.iter_mut().find(|b| b.bang == name) {
                    Some(bang) => {
                        bang.sources.push(source.name().to_string());
                        bang.on_request &= on_request;
                    }
                    None => bangs.push(Bang {
                        bang: name,
                        sources: vec![source.name().to_string()],
                        on_request,
                    }),
                }
            }
        }
        bangs.sort_by(|a, b| a.bang.cmp(&b.bang));
        bangs
    }

    /// Ticket of the most recent [`search`](Self::search).
    pub fn latest_ticket(&self) -> u64 {
        self.tickets.load(Ordering::Relaxed)
    }
}

/// Live sources return items best-first; turn that order into scores.
fn rank(items: Vec<Resource>, query: &Query) -> Vec<Hit> {
    items
        .into_iter()
        .filter(|r| query.accepts(r))
        .take(query.limit)
        .enumerate()
        .map(|(i, r)| Hit {
            resource: Arc::new(r),
            score: 1.0 / (i as f32 + 1.0),
        })
        .collect()
}

fn read<T: ?Sized>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T: ?Sized>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use lantern_resource::{Capabilities, Kind, Location, ResourceId};

    use super::*;

    struct Fake {
        id: SourceId,
        caps: Capabilities,
        bangs: Vec<&'static str>,
        items: Mutex<Result<Vec<String>, String>>,
    }

    impl Fake {
        fn new(id: &str, caps: Capabilities, titles: &[&str]) -> Arc<Self> {
            Arc::new(Self {
                id: SourceId::new(id).unwrap(),
                caps,
                bangs: Vec::new(),
                items: Mutex::new(Ok(titles.iter().map(|t| (*t).to_owned()).collect())),
            })
        }

        fn banged(id: &str, caps: Capabilities, bang: &'static str, titles: &[&str]) -> Arc<Self> {
            Self::with_bangs(id, caps, vec![bang], titles)
        }

        fn with_bangs(
            id: &str,
            caps: Capabilities,
            bangs: Vec<&'static str>,
            titles: &[&str],
        ) -> Arc<Self> {
            let mut fake = Arc::into_inner(Self::new(id, caps, titles)).unwrap();
            fake.bangs = bangs;
            Arc::new(fake)
        }

        fn fail(&self) {
            *self.items.lock().unwrap() = Err("offline".into());
        }

        fn items(&self) -> Result<Vec<Resource>, SourceError> {
            let items = self
                .items
                .lock()
                .unwrap()
                .clone()
                .map_err(SourceError::Unavailable)?;
            Ok(items
                .into_iter()
                .map(|t| {
                    Resource::new(
                        ResourceId::new(&self.id, &t),
                        t.clone(),
                        Kind::Image,
                        Location::Url(t),
                    )
                })
                .collect())
        }
    }

    impl Source for Fake {
        fn id(&self) -> &SourceId {
            &self.id
        }
        fn name(&self) -> &str {
            "fake"
        }
        fn capabilities(&self) -> Capabilities {
            self.caps
        }
        fn bangs(&self) -> Vec<&str> {
            self.bangs.clone()
        }
        fn scan(&self) -> Result<Vec<Resource>, SourceError> {
            self.items()
        }
        fn search(&self, _: &Query) -> Result<Vec<Resource>, SourceError> {
            self.items()
        }
    }

    #[test]
    fn rejects_duplicate_ids() {
        let searcher = Searcher::new();
        searcher
            .register(Fake::new("a", Capabilities::INDEXED, &[]))
            .unwrap();
        assert!(matches!(
            searcher.register(Fake::new("a", Capabilities::LIVE, &[])),
            Err(SearchError::DuplicateSource(_))
        ));
    }

    #[test]
    fn reindex_populates_and_failures_keep_old_entries() {
        let searcher = Searcher::new();
        let local = Fake::new("local", Capabilities::INDEXED, &["sunset", "sunrise"]);
        searcher.register(local.clone()).unwrap();

        let reports = searcher.reindex_all();
        assert_eq!(reports.len(), 1);
        assert_eq!(*reports[0].result.as_ref().unwrap(), 2);
        assert_eq!(searcher.search_index(&Query::new("sun")).len(), 2);

        local.fail();
        assert!(searcher.reindex_all()[0].result.is_err());
        assert_eq!(searcher.search_index(&Query::new("sun")).len(), 2);
    }

    #[test]
    fn stream_has_index_first_then_live_sources() {
        let searcher = Searcher::new();
        searcher
            .register(Fake::new("local", Capabilities::INDEXED, &["cross"]))
            .unwrap();
        searcher
            .register(Fake::new(
                "web",
                Capabilities::LIVE,
                &["cross photo", "crown"],
            ))
            .unwrap();
        let broken = Fake::new("broken", Capabilities::LIVE, &[]);
        broken.fail();
        searcher.register(broken).unwrap();
        searcher.reindex_all();

        let mut stream = searcher.search(Query::new("cross"));
        assert_eq!(stream.ticket(), searcher.latest_ticket());

        let Some(SearchEvent::Hits {
            origin: Origin::Index,
            hits,
        }) = stream.next()
        else {
            panic!("index results must come first");
        };
        assert_eq!(hits.len(), 1);

        let mut live_hits = 0;
        let mut failures = 0;
        for event in stream {
            match event {
                SearchEvent::Hits {
                    origin: Origin::Source(id),
                    hits,
                } => {
                    assert_eq!(id.as_str(), "web");
                    assert!(hits[0].score > hits[1].score);
                    live_hits += hits.len();
                }
                SearchEvent::Failed { source, .. } => {
                    assert_eq!(source.as_str(), "broken");
                    failures += 1;
                }
                SearchEvent::Hits {
                    origin: Origin::Index,
                    ..
                } => panic!("index answers once"),
            }
        }
        assert_eq!((live_hits, failures), (2, 1));
    }

    #[test]
    fn source_filter_skips_live_sources() {
        let searcher = Searcher::new();
        searcher
            .register(Fake::new("web", Capabilities::LIVE, &["x"]))
            .unwrap();
        let query = Query::new("x").with_source(SourceId::new("local").unwrap());
        assert_eq!(searcher.search(query).count(), 1); // only the (empty) index batch
    }

    #[test]
    fn an_on_request_source_answers_only_when_named() {
        let searcher = Searcher::new();
        let caps = Capabilities::ON_REQUEST;
        searcher
            .register(Fake::banged("yt", caps, "yt", &["x"]))
            .unwrap();
        assert_eq!(searcher.search(Query::new("x")).count(), 1, "index only");
        let parsed = searcher.parse("!yt x");
        assert!(matches!(&parsed.scope, Scope::Bang(bang) if bang.on_request));
        assert_eq!(searcher.search(parsed.query).count(), 2);
    }

    #[test]
    fn unregister_drops_indexed_items() {
        let searcher = Searcher::new();
        searcher
            .register(Fake::new("local", Capabilities::INDEXED, &["a"]))
            .unwrap();
        searcher.reindex_all();
        searcher
            .unregister(&SourceId::new("local").unwrap())
            .unwrap();
        assert!(searcher.index().is_empty());
    }

    fn bible_and_folders() -> Searcher {
        let searcher = Searcher::new();
        for (id, bang) in [("rv1909", "b"), ("kjv", "B"), ("local", "f")] {
            let caps = Capabilities::INDEXED;
            searcher
                .register(Fake::banged(id, caps, bang, &[&format!("cross {id}")]))
                .unwrap();
        }
        searcher.reindex_all();
        searcher
    }

    fn titles(searcher: &Searcher, query: &Query) -> Vec<String> {
        let mut titles: Vec<String> = searcher
            .search_index(query)
            .into_iter()
            .map(|hit| hit.resource.title.clone())
            .collect();
        titles.sort();
        titles
    }

    #[test]
    fn a_bang_limits_the_query_to_its_sources() {
        let searcher = bible_and_folders();
        let parsed = searcher.parse("!B cross");
        assert_eq!(parsed.query.text, "cross");
        let Scope::Bang(bang) = &parsed.scope else {
            panic!("{parsed:?}");
        };
        assert_eq!((bang.bang.as_str(), bang.sources.len()), ("b", 2));
        assert_eq!(parsed.query.sources.len(), 2);
        assert_eq!(
            titles(&searcher, &parsed.query),
            ["cross kjv", "cross rv1909"]
        );

        let everything = searcher.parse(" cross ");
        assert_eq!(everything.scope, Scope::Everything);
        assert_eq!(titles(&searcher, &everything.query).len(), 3);
    }

    #[test]
    fn an_unknown_bang_asks_no_one() {
        let searcher = bible_and_folders();
        let parsed = searcher.parse("!zz cross");
        assert_eq!(parsed.scope, Scope::Unknown("zz".into()));
    }

    #[test]
    fn bangs_are_listed_once_with_every_source() {
        let listed: Vec<(String, usize)> = bible_and_folders()
            .bangs()
            .into_iter()
            .map(|b| (b.bang, b.sources.len()))
            .collect();
        assert_eq!(listed, [("b".to_string(), 2), ("f".to_string(), 1)]);
    }

    #[test]
    fn a_folder_with_its_own_bang_answers_it_and_the_shared_one() {
        let searcher = bible_and_folders();
        searcher
            .register(Fake::with_bangs(
                "local-h",
                Capabilities::INDEXED,
                vec!["f", "h"],
                &["cross hymn"],
            ))
            .unwrap();
        searcher.reindex_all();
        let listed: Vec<(String, usize)> = searcher
            .bangs()
            .into_iter()
            .map(|b| (b.bang, b.sources.len()))
            .collect();
        assert_eq!(listed, [("b".into(), 2), ("f".into(), 2), ("h".into(), 1)]);
        let hymns = searcher.parse("!h cross").query;
        assert_eq!(titles(&searcher, &hymns), ["cross hymn"]);
        let folders = searcher.parse("!F cross").query;
        assert_eq!(titles(&searcher, &folders), ["cross hymn", "cross local"]);
    }
}
