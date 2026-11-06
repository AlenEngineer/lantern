use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Bound;
use std::sync::Arc;

use chorquest_resource::{Hit, Location, Query, Resource, ResourceId, SourceId};

use crate::normalize::tokens;

// Field weights: a word in the title matters more than one in the lyrics.
const TITLE: u8 = 3;
const LABEL: u8 = 2;
const BODY: u8 = 1;

// Match quality multipliers.
const EXACT: f32 = 2.0;
const PREFIX: f32 = 1.0;
const FUZZY: f32 = 0.5;

/// Bonus when the whole query is the beginning of the title.
const TITLE_PREFIX_BONUS: f32 = 4.0;
/// Bonus when the whole query appears somewhere in the title.
const TITLE_PHRASE_BONUS: f32 = 2.0;

/// Words shorter than this never match fuzzily (too many false positives).
const FUZZY_MIN_CHARS: usize = 4;

type DocId = u32;

struct Doc {
    resource: Arc<Resource>,
    /// Title tokens joined by spaces, for phrase bonuses and stable ordering.
    title_key: String,
    /// Every term this doc was indexed under, to remove it later.
    terms: Vec<String>,
}

/// An in-memory inverted index of resources.
///
/// Terms are kept in a sorted map so prefix lookups are a range scan. Each
/// term points to the docs containing it and the weight of the field it was
/// found in.
///
/// The index is not synchronized; [`Searcher`](crate::Searcher) wraps it in a lock.
#[derive(Default)]
pub struct Index {
    docs: Vec<Option<Doc>>,
    free: Vec<DocId>,
    by_id: HashMap<ResourceId, DocId>,
    terms: BTreeMap<String, Vec<(DocId, u8)>>,
}

impl Index {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of indexed resources.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether nothing is indexed.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Looks a resource up by id.
    pub fn get(&self, id: &ResourceId) -> Option<Arc<Resource>> {
        let slot = *self.by_id.get(id)?;
        self.doc(slot).map(|d| Arc::clone(&d.resource))
    }

    /// Every indexed resource, in no particular order.
    pub fn resources(&self) -> impl Iterator<Item = &Arc<Resource>> {
        self.docs.iter().flatten().map(|d| &d.resource)
    }

    /// Adds a resource, replacing any previous one with the same id.
    pub fn insert(&mut self, resource: Resource) {
        self.remove(&resource.id);

        let weighted = field_terms(&resource);
        let slot = match self.free.pop() {
            Some(slot) => slot,
            None => {
                self.docs.push(None);
                DocId::try_from(self.docs.len() - 1).expect("more than u32::MAX resources")
            }
        };
        for (term, weight) in &weighted {
            self.terms
                .entry(term.clone())
                .or_default()
                .push((slot, *weight));
        }
        self.by_id.insert(resource.id.clone(), slot);
        self.docs[slot as usize] = Some(Doc {
            title_key: tokens(&resource.title).join(" "),
            terms: weighted.into_keys().collect(),
            resource: Arc::new(resource),
        });
    }

    /// Removes a resource. Returns whether it was there.
    pub fn remove(&mut self, id: &ResourceId) -> bool {
        let Some(slot) = self.by_id.remove(id) else {
            return false;
        };
        if let Some(doc) = self.docs[slot as usize].take() {
            for term in &doc.terms {
                if let Some(postings) = self.terms.get_mut(term) {
                    postings.retain(|(d, _)| *d != slot);
                    if postings.is_empty() {
                        self.terms.remove(term);
                    }
                }
            }
        }
        self.free.push(slot);
        true
    }

    /// Removes everything that belongs to `source`. Returns how many were removed.
    pub fn remove_source(&mut self, source: &SourceId) -> usize {
        let doomed: HashSet<DocId> = self
            .by_id
            .iter()
            .filter(|(id, _)| id.belongs_to(source))
            .map(|(_, slot)| *slot)
            .collect();
        if doomed.is_empty() {
            return 0;
        }
        // One pass over all terms beats one pass per removed doc.
        self.by_id.retain(|_, slot| !doomed.contains(slot));
        self.terms.retain(|_, postings| {
            postings.retain(|(d, _)| !doomed.contains(d));
            !postings.is_empty()
        });
        for &slot in &doomed {
            self.docs[slot as usize] = None;
            self.free.push(slot);
        }
        doomed.len()
    }

    /// Replaces everything `source` had with `resources`. Returns the new count.
    pub fn replace_source(
        &mut self,
        source: &SourceId,
        resources: impl IntoIterator<Item = Resource>,
    ) -> usize {
        self.remove_source(source);
        let mut count = 0;
        for resource in resources {
            self.insert(resource);
            count += 1;
        }
        count
    }

    /// Searches the index.
    ///
    /// Every word of the query must match (as a whole word, a prefix, or with
    /// one typo). An empty query returns everything that passes the filters,
    /// alphabetically, which is how the dashboard can browse e.g. "all videos".
    pub fn search(&self, query: &Query) -> Vec<Hit> {
        let words = tokens(&query.text);

        let candidates: Vec<(DocId, f32)> = if words.is_empty() {
            self.by_id.values().map(|&slot| (slot, 0.0)).collect()
        } else {
            let mut acc: Option<HashMap<DocId, f32>> = None;
            for word in &words {
                let mut matches = self.match_word(word);
                let merged = match acc {
                    None => matches,
                    Some(prev) => prev
                        .into_iter()
                        .filter_map(|(d, s)| matches.remove(&d).map(|s2| (d, s + s2)))
                        .collect(),
                };
                if merged.is_empty() {
                    return Vec::new();
                }
                acc = Some(merged);
            }
            acc.unwrap_or_default().into_iter().collect()
        };

        let phrase = words.join(" ");
        let mut ranked: Vec<(f32, &Doc)> = candidates
            .into_iter()
            .filter_map(|(slot, score)| {
                let doc = self.doc(slot)?;
                if !query.accepts(&doc.resource) {
                    return None;
                }
                let bonus = if phrase.is_empty() {
                    0.0
                } else if doc.title_key.starts_with(&phrase) {
                    TITLE_PREFIX_BONUS
                } else if doc.title_key.contains(&phrase) {
                    TITLE_PHRASE_BONUS
                } else {
                    0.0
                };
                Some((score + bonus, doc))
            })
            .collect();

        ranked.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| a.1.title_key.cmp(&b.1.title_key))
        });
        ranked.truncate(query.limit);
        ranked
            .into_iter()
            .map(|(score, doc)| Hit {
                resource: Arc::clone(&doc.resource),
                score,
            })
            .collect()
    }

    /// Best score per doc for a single query word.
    fn match_word(&self, word: &str) -> HashMap<DocId, f32> {
        let mut best = HashMap::new();
        let prefixed = self
            .terms
            .range::<str, _>((Bound::Included(word), Bound::Unbounded))
            .take_while(|(term, _)| term.starts_with(word));
        for (term, postings) in prefixed {
            let quality = if term == word { EXACT } else { PREFIX };
            for &(slot, weight) in postings {
                bump(&mut best, slot, quality * f32::from(weight));
            }
        }

        // Only pay for a full scan when nothing matched properly.
        if best.is_empty() && word.chars().count() >= FUZZY_MIN_CHARS {
            for (term, postings) in &self.terms {
                if fuzzy(word, term) {
                    for &(slot, weight) in postings {
                        bump(&mut best, slot, FUZZY * f32::from(weight));
                    }
                }
            }
        }
        best
    }

    fn doc(&self, slot: DocId) -> Option<&Doc> {
        self.docs.get(slot as usize)?.as_ref()
    }
}

fn bump(best: &mut HashMap<DocId, f32>, slot: DocId, score: f32) {
    let entry = best.entry(slot).or_insert(0.0);
    if score > *entry {
        *entry = score;
    }
}

/// Every searchable term of a resource with the weight of the best field it appears in.
fn field_terms(resource: &Resource) -> HashMap<String, u8> {
    fn add(out: &mut HashMap<String, u8>, text: &str, weight: u8) {
        for term in tokens(text) {
            let entry = out.entry(term).or_insert(0);
            *entry = (*entry).max(weight);
        }
    }

    let mut out = HashMap::new();
    add(&mut out, &resource.title, TITLE);
    if let Some(subtitle) = &resource.subtitle {
        add(&mut out, subtitle, LABEL);
    }
    for tag in &resource.tags {
        add(&mut out, tag, LABEL);
    }
    if let Location::Inline(body) = &resource.location {
        add(&mut out, body, BODY);
    }
    out
}

/// Whether `word` is one typo away from `term`, or from the beginning of
/// `term` (the operator may still be typing).
fn fuzzy(word: &str, term: &str) -> bool {
    if within_one_edit(word, term) {
        return true;
    }
    let n = word.chars().count();
    let prefix_end = term.char_indices().nth(n).map_or(term.len(), |(i, _)| i);
    prefix_end < term.len() && within_one_edit(word, &term[..prefix_end])
}

/// One insertion, deletion, substitution or swap of adjacent letters.
fn within_one_edit(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (short, long) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    match long.len() - short.len() {
        0 => {
            let diffs: Vec<usize> = (0..short.len()).filter(|&i| short[i] != long[i]).collect();
            match diffs.as_slice() {
                [] | [_] => true,
                [i, j] => *j == i + 1 && short[*i] == long[*j] && short[*j] == long[*i],
                _ => false,
            }
        }
        1 => {
            let same = short
                .iter()
                .zip(long.iter())
                .take_while(|(x, y)| x == y)
                .count();
            short[same..] == long[same + 1..]
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use chorquest_resource::Kind;

    use super::*;

    fn source(id: &str) -> SourceId {
        SourceId::new(id).unwrap()
    }

    fn file(src: &str, title: &str, kind: Kind) -> Resource {
        let path = format!("/{title}");
        Resource::new(
            ResourceId::new(&source(src), &path),
            title,
            kind,
            Location::File(path.into()),
        )
    }

    fn titles(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|h| h.resource.title.as_str()).collect()
    }

    fn sample() -> Index {
        let mut index = Index::new();
        index.insert(file("local", "Canción de alabanza", Kind::Audio));
        index.insert(file("local", "Aleluya", Kind::Audio));
        index.insert(file("local", "Video de bienvenida", Kind::Video));
        index.insert(file("local", "Anuncios", Kind::Presentation).with_tag("Domingo"));
        index
    }

    #[test]
    fn ignores_accents_and_case() {
        let index = sample();
        assert_eq!(
            titles(&index.search(&Query::new("CANCION"))),
            ["Canción de alabanza"]
        );
    }

    #[test]
    fn matches_prefixes_while_typing() {
        let index = sample();
        assert_eq!(titles(&index.search(&Query::new("alel"))), ["Aleluya"]);
    }

    #[test]
    fn every_word_must_match() {
        let index = sample();
        assert_eq!(
            titles(&index.search(&Query::new("de bienv"))),
            ["Video de bienvenida"]
        );
        assert!(index.search(&Query::new("aleluya bienvenida")).is_empty());
    }

    #[test]
    fn forgives_one_typo() {
        let index = sample();
        assert_eq!(titles(&index.search(&Query::new("aleluia"))), ["Aleluya"]);
        assert_eq!(titles(&index.search(&Query::new("anucnios"))), ["Anuncios"]);
    }

    #[test]
    fn searches_tags_and_inline_text() {
        let mut index = sample();
        index.insert(Resource::new(
            ResourceId::new(&source("hymnal"), "1"),
            "Santo, santo, santo",
            Kind::Text,
            Location::Inline("¡Santo, santo, santo!\nSeñor omnipotente".into()),
        ));
        assert_eq!(titles(&index.search(&Query::new("domingo"))), ["Anuncios"]);
        assert_eq!(
            titles(&index.search(&Query::new("omnipotente"))),
            ["Santo, santo, santo"]
        );
    }

    #[test]
    fn title_matches_rank_first() {
        let mut index = sample();
        index.insert(file("local", "Otra", Kind::Audio).with_tag("aleluya"));
        assert_eq!(
            titles(&index.search(&Query::new("aleluya"))),
            ["Aleluya", "Otra"]
        );
    }

    #[test]
    fn empty_query_browses_with_filters() {
        let index = sample();
        let hits = index.search(&Query::new("").with_kind(Kind::Audio));
        assert_eq!(titles(&hits), ["Aleluya", "Canción de alabanza"]);
    }

    #[test]
    fn respects_limit() {
        let index = sample();
        assert_eq!(index.search(&Query::new("").with_limit(2)).len(), 2);
    }

    #[test]
    fn replacing_a_source_leaves_others_alone() {
        let mut index = sample();
        index.insert(file("other", "Aleluya remix", Kind::Audio));
        let count = index.replace_source(&source("local"), [file("local", "Nuevo", Kind::Image)]);
        assert_eq!(count, 1);
        assert_eq!(index.len(), 2);
        assert_eq!(
            titles(&index.search(&Query::new("aleluya"))),
            ["Aleluya remix"]
        );
        assert_eq!(titles(&index.search(&Query::new("nuevo"))), ["Nuevo"]);
    }

    #[test]
    fn reinserting_replaces() {
        let mut index = Index::new();
        index.insert(file("local", "Uno", Kind::Image));
        let mut changed = file("local", "Uno", Kind::Image);
        changed.title = "Dos".into();
        index.insert(changed);
        assert_eq!(index.len(), 1);
        assert!(index.search(&Query::new("uno")).is_empty());
        assert_eq!(index.search(&Query::new("dos")).len(), 1);
    }

    #[test]
    fn edit_distance() {
        assert!(within_one_edit("abcd", "abcd"));
        assert!(within_one_edit("abcd", "abxd"));
        assert!(within_one_edit("abcd", "abd"));
        assert!(within_one_edit("abcd", "abcde"));
        assert!(within_one_edit("abcd", "bacd"));
        assert!(!within_one_edit("abcd", "badc"));
        assert!(!within_one_edit("abcd", "ab"));
    }
}
