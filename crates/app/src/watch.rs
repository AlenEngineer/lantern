//! Watching the media folders, so a file added, removed or renamed shows up
//! in search without a reindex. Each folder's source is scanned again once
//! its folder has been quiet for a moment, so copying fifty hymns is one
//! scan. Reading a file (a thumbnail, playing it) changes nothing, and is
//! never a reason to scan.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

use lantern_engine::{Engine, SourceId};
use notify::event::ModifyKind;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::log;

/// How long a folder stays quiet before its source is scanned again.
pub const QUIET: Duration = Duration::from_millis(800);

/// Watches folders while it lives; dropping it stops watching.
pub struct Watch {
    _watcher: RecommendedWatcher,
}

/// Starts watching each folder, scanning its source again after changes
/// and counting each scan in `changes`, so a search on screen can follow.
/// A folder that can't be watched (missing, an unplugged drive) is logged
/// and skipped. `None` if the system can't watch at all.
pub fn start(
    folders: Vec<(PathBuf, SourceId)>,
    engine: Arc<Engine>,
    changes: Arc<AtomicU64>,
) -> Option<Watch> {
    let (events, received) = channel::<notify::Result<Event>>();
    let mut watcher = match notify::recommended_watcher(events) {
        Ok(watcher) => watcher,
        Err(err) => {
            log(format!("watch: can't watch folders: {err}"));
            return None;
        }
    };
    let mut watched = Vec::new();
    for (folder, source) in folders {
        match watcher.watch(&folder, RecursiveMode::Recursive) {
            Ok(()) => watched.push((folder, source)),
            Err(err) => log(format!("watch: {}: {err}", folder.display())),
        }
    }
    if watched.is_empty() {
        return None;
    }
    std::thread::spawn(move || {
        // Ends when the watcher is dropped, which drops the sender.
        while let Ok(first) = received.recv() {
            let mut changed = sources_changed(&first, &watched);
            loop {
                match received.recv_timeout(QUIET) {
                    Ok(event) => changed.extend(sources_changed(&event, &watched)),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            for source in changed {
                match engine.searcher().reindex(&source) {
                    Ok(count) => {
                        log(format!("watch: {source} changed, {count} items"));
                        changes.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(err) => log(format!("watch: {source}: {err}")),
                }
            }
        }
    });
    Some(Watch { _watcher: watcher })
}

/// The sources whose folders an event changed: files added, removed,
/// renamed or written. Reads and metadata changes (a thumbnail, playback)
/// change nothing.
fn sources_changed(
    event: &notify::Result<Event>,
    watched: &[(PathBuf, SourceId)],
) -> BTreeSet<SourceId> {
    let Ok(event) = event else {
        return BTreeSet::new();
    };
    let changes = match event.kind {
        EventKind::Create(_) | EventKind::Remove(_) => true,
        EventKind::Modify(ModifyKind::Metadata(_)) => false,
        EventKind::Modify(_) => true,
        EventKind::Access(_) | EventKind::Any | EventKind::Other => false,
    };
    if !changes {
        return BTreeSet::new();
    }
    event
        .paths
        .iter()
        .flat_map(|path| owners(path, watched))
        .collect()
}

fn owners<'a>(
    path: &'a Path,
    watched: &'a [(PathBuf, SourceId)],
) -> impl Iterator<Item = SourceId> + 'a {
    watched
        .iter()
        .filter(move |(folder, _)| path.starts_with(folder))
        .map(|(_, source)| source.clone())
}

#[cfg(test)]
mod tests {
    use notify::event::{AccessKind, CreateKind, MetadataKind, RenameMode};

    use super::*;

    fn event(kind: EventKind, path: &str) -> notify::Result<Event> {
        Ok(Event::new(kind).add_path(PathBuf::from(path)))
    }

    #[test]
    fn only_adding_removing_renaming_or_writing_counts() {
        let hymns = SourceId::new("local-h").unwrap();
        let videos = SourceId::new("local").unwrap();
        let watched = vec![
            (PathBuf::from("/media/Himnos"), hymns.clone()),
            (PathBuf::from("/media/Videos"), videos.clone()),
        ];
        let changed = |kind, path| sources_changed(&event(kind, path), &watched);

        assert_eq!(
            changed(EventKind::Create(CreateKind::File), "/media/Himnos/436.pdf"),
            BTreeSet::from([hymns.clone()])
        );
        assert_eq!(
            changed(
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                "/media/Videos/a.mp4"
            ),
            BTreeSet::from([videos])
        );
        assert!(changed(EventKind::Access(AccessKind::Read), "/media/Himnos/436.pdf").is_empty());
        assert!(
            changed(
                EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)),
                "/media/Himnos/1.pdf"
            )
            .is_empty()
        );
        assert!(changed(EventKind::Create(CreateKind::File), "/elsewhere/x.pdf").is_empty());
        assert!(sources_changed(&Err(notify::Error::generic("gone")), &watched).is_empty());
    }
}
