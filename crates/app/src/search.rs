//! The dashboard's search box: what was typed, the results grouped by
//! source, and which result is picked.
//!
//! Every edit searches the in-memory index at once. The live sources (links,
//! Bible references) run after a pause, through [`Engine::search`], and only
//! the latest live search's events are kept.

use std::sync::Arc;

use chorquest_engine::{Engine, Hit, Kind, Origin, Query, Resource, ResourceId, SearchEvent};

/// Results from one source, under its display name.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    source: String,
    pub hits: Vec<Arc<Resource>>,
    /// Why the source failed, if it did.
    pub error: Option<String>,
}

/// Where the live part of the current query stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Live {
    /// Nothing to search, or the search finished.
    Idle,
    /// Waiting out the pause after edit `n`.
    Waiting(u64),
    /// Live search `ticket` is running.
    Running(u64),
}

#[derive(Debug)]
pub struct Search {
    pub query: String,
    groups: Vec<Group>,
    /// Kept by id, not position, so live results arriving above it don't
    /// move the pick to another row.
    picked: Option<ResourceId>,
    edits: u64,
    pub live: Live,
}

impl Default for Search {
    fn default() -> Self {
        Search {
            query: String::new(),
            groups: Vec::new(),
            picked: None,
            edits: 0,
            live: Live::Idle,
        }
    }
}

/// The dashboard can't show web pages yet, so they can't be picked.
pub fn supported(resource: &Resource) -> bool {
    resource.kind != Kind::Web
}

impl Search {
    /// The operator changed the query. Shows index results right away and
    /// returns the edit number to start the live search with after a pause.
    pub fn edit(&mut self, query: String, engine: &Engine) -> Option<u64> {
        self.query = query;
        self.edits += 1;
        self.picked = None;
        self.groups.clear();
        if self.query.trim().is_empty() {
            self.live = Live::Idle;
            return None;
        }
        let names = source_names(engine);
        for hit in engine.search_index(&Query::new(&self.query)) {
            let source = hit.resource.id.source().to_string();
            self.group(&source, &names).hits.push(hit.resource);
        }
        self.live = Live::Waiting(self.edits);
        Some(self.edits)
    }

    /// The pause after edit `edit` is over. Starts the live search unless
    /// the operator kept typing.
    pub fn start_live(
        &mut self,
        edit: u64,
        engine: &Engine,
    ) -> Option<chorquest_engine::SearchStream> {
        if self.live != Live::Waiting(edit) {
            return None;
        }
        let stream = engine.search(Query::new(&self.query));
        self.live = Live::Running(stream.ticket());
        Some(stream)
    }

    /// An event from live search `ticket`. Ignored unless it is the latest.
    pub fn live_event(&mut self, ticket: u64, event: &SearchEvent, engine: &Engine) {
        if self.live != Live::Running(ticket) {
            return;
        }
        match event {
            // The edit already showed the index's results for this query.
            SearchEvent::Hits {
                origin: Origin::Index,
                ..
            } => {}
            SearchEvent::Hits {
                origin: Origin::Source(source),
                hits,
            } => self.merge(source.as_str(), hits, &source_names(engine)),
            SearchEvent::Failed { source, error } => {
                self.group(source.as_str(), &source_names(engine)).error = Some(error.clone());
            }
        }
    }

    /// Live search `ticket` has heard from every source.
    pub fn live_done(&mut self, ticket: u64) {
        if self.live == Live::Running(ticket) {
            self.live = Live::Idle;
        }
    }

    /// A live source's hits go first in its group: a parsed Bible reference
    /// or a pasted link is exactly what was asked for.
    fn merge(&mut self, source: &str, hits: &[Hit], names: &[(String, String)]) {
        if hits.is_empty() {
            return;
        }
        let group = self.group(source, names);
        let fresh: Vec<Arc<Resource>> = hits.iter().map(|hit| hit.resource.clone()).collect();
        group
            .hits
            .retain(|old| !fresh.iter().any(|new| new.id == old.id));
        group.hits.splice(0..0, fresh);
    }

    fn group(&mut self, source: &str, names: &[(String, String)]) -> &mut Group {
        let index = match self.groups.iter().position(|g| g.source == source) {
            Some(index) => index,
            None => {
                let name = names
                    .iter()
                    .find(|(id, _)| id == source)
                    .map_or(source, |(_, name)| name.as_str());
                self.groups.push(Group {
                    name: name.to_string(),
                    source: source.to_string(),
                    hits: Vec::new(),
                    error: None,
                });
                self.groups.len() - 1
            }
        };
        &mut self.groups[index]
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    pub fn picked(&self) -> Option<&Arc<Resource>> {
        let id = self.picked.as_ref()?;
        self.results().find(|r| &r.id == id)
    }

    pub fn is_picked(&self, resource: &Resource) -> bool {
        self.picked.as_ref() == Some(&resource.id)
    }

    /// Picks a result, if it can be shown. Returns it.
    pub fn pick(&mut self, id: &ResourceId) -> Option<Arc<Resource>> {
        let resource = self
            .results()
            .find(|r| &r.id == id && supported(r))?
            .clone();
        self.picked = Some(resource.id.clone());
        Some(resource)
    }

    /// Picks the `n`th result that can be shown, counted from 0.
    pub fn pick_nth(&mut self, n: usize) -> Option<Arc<Resource>> {
        let id = self.pickable().nth(n)?.id.clone();
        self.pick(&id)
    }

    /// Picks the result after (or before) the picked one, stopping at the ends.
    pub fn step(&mut self, forward: bool) -> Option<Arc<Resource>> {
        let pickable: Vec<&Arc<Resource>> = self.pickable().collect();
        let at = self
            .picked
            .as_ref()
            .and_then(|id| pickable.iter().position(|r| &r.id == id));
        let next = match (at, forward) {
            (None, _) => 0,
            (Some(i), true) => (i + 1).min(pickable.len().saturating_sub(1)),
            (Some(i), false) => i.saturating_sub(1),
        };
        let id = pickable.get(next)?.id.clone();
        self.pick(&id)
    }

    /// Forgets the pick, e.g. once it went on program.
    pub fn unpick(&mut self) {
        self.picked = None;
    }

    /// Where the picked result is in the list, from 0.0 (top) to 1.0, for scrolling to it.
    pub fn picked_offset(&self) -> Option<f32> {
        let id = self.picked.as_ref()?;
        let rows = self.results().count();
        let at = self.results().position(|r| &r.id == id)?;
        Some(if rows > 1 {
            at as f32 / (rows - 1) as f32
        } else {
            0.0
        })
    }

    fn results(&self) -> impl Iterator<Item = &Arc<Resource>> {
        self.groups.iter().flat_map(|g| g.hits.iter())
    }

    fn pickable(&self) -> impl Iterator<Item = &Arc<Resource>> {
        self.results().filter(|r| supported(r))
    }
}

/// (id, display name) of every registered source.
fn source_names(engine: &Engine) -> Vec<(String, String)> {
    engine
        .searcher()
        .sources()
        .iter()
        .map(|s| (s.id().to_string(), s.name().to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use super::*;
    use crate::config::Config;
    use crate::library::Library;

    fn engine() -> Arc<Engine> {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let config = Config::parse(
            r#"
            [[folders]]
            path = "scripts/e2e/fixtures"
            [[bibles]]
            lang = "es"
            path = "sources/bible/fixtures/sample.tsv"
            name = "RV1909"
            "#,
            repo,
            None,
        )
        .unwrap();
        let engine = Arc::new(Engine::new());
        let library = Library::start(engine.clone(), config);
        assert!(library.settle(Duration::from_secs(30)));
        engine
    }

    fn names(search: &Search) -> Vec<(String, Vec<String>)> {
        search
            .groups()
            .iter()
            .map(|g| {
                let titles = g.hits.iter().map(|r| r.title.clone()).collect();
                (g.name.clone(), titles)
            })
            .collect()
    }

    fn run_live(search: &mut Search, edit: u64, engine: &Engine) {
        let stream = search.start_live(edit, engine).unwrap();
        let ticket = stream.ticket();
        for event in stream {
            search.live_event(ticket, &event, engine);
        }
        search.live_done(ticket);
    }

    #[test]
    fn index_results_are_grouped_by_source_name_at_once() {
        let engine = engine();
        let mut search = Search::default();
        search.edit("pa".into(), &engine);
        let groups = names(&search);
        let local = groups.iter().find(|(name, _)| name == "Local folders");
        assert!(
            local.unwrap().1.contains(&"pages".to_string()),
            "{groups:?}"
        );
        assert!(
            groups.iter().any(|(name, _)| name == "RV1909"),
            "{groups:?}"
        );
    }

    #[test]
    fn a_live_reference_goes_first_in_its_group() {
        let engine = engine();
        let mut search = Search::default();
        let edit = search.edit("juan 3:16-17".into(), &engine).unwrap();
        run_live(&mut search, edit, &engine);
        let groups = names(&search);
        let bible = groups.iter().find(|(name, _)| name == "RV1909").unwrap();
        assert_eq!(bible.1[0], "Juan 3:16-17", "{groups:?}");
        assert_eq!(search.live, Live::Idle);
    }

    #[test]
    fn a_pasted_link_appears_under_links_after_the_pause() {
        let engine = engine();
        let mut search = Search::default();
        let edit = search
            .edit("https://youtu.be/jNQXAC9IVRw".into(), &engine)
            .unwrap();
        assert!(search.groups().is_empty(), "nothing before the pause");
        run_live(&mut search, edit, &engine);
        let link = (
            "Links".to_string(),
            vec!["YouTube video jNQXAC9IVRw".to_string()],
        );
        assert_eq!(names(&search), [link]);
    }

    #[test]
    fn stale_live_results_are_dropped() {
        let engine = engine();
        let mut search = Search::default();
        let old = search
            .edit("https://youtu.be/jNQXAC9IVRw".into(), &engine)
            .unwrap();
        let stream = search.start_live(old, &engine).unwrap();
        let ticket = stream.ticket();
        let late: Vec<SearchEvent> = stream.collect();

        let newer = search.edit("pa".into(), &engine).unwrap();
        assert!(
            search.start_live(old, &engine).is_none(),
            "the old pause is over"
        );
        let before = names(&search);
        for event in &late {
            search.live_event(ticket, event, &engine);
        }
        assert_eq!(names(&search), before);
        assert_eq!(search.live, Live::Waiting(newer));
    }

    #[test]
    fn picking_skips_web_pages_and_survives_new_rows() {
        let engine = engine();
        let mut search = Search::default();
        let edit = search.edit("pa".into(), &engine).unwrap();
        let web = search
            .groups()
            .iter()
            .flat_map(|g| &g.hits)
            .find(|r| r.kind == Kind::Web)
            .expect("page.html matches")
            .id
            .clone();
        assert!(search.pick(&web).is_none());
        let stepped: Vec<Arc<Resource>> =
            std::iter::from_fn(|| search.step(true)).take(30).collect();
        assert!(stepped.len() > 2);
        assert!(stepped.iter().all(|r| r.kind != Kind::Web), "{stepped:?}");

        let first = search.pick_nth(0).unwrap();
        run_live(&mut search, edit, &engine);
        assert_eq!(search.picked().map(|r| &r.id), Some(&first.id));
    }
}
