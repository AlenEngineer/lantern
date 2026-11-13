//! Completing a path as it's typed, the way a shell's Tab does: what's in
//! the folder typed so far that starts with the rest, ignoring case and
//! accents (`musica` finds *Música*). Folders end in `/`, so choosing one
//! lists what's inside it next.

use std::path::{Path, PathBuf};

use lantern_engine::fold;

/// How many suggestions to offer at once.
pub const MAX: usize = 8;

/// What a path field takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    /// Folders only (media folders).
    Folders,
    /// Folders to look through, and files with one of these extensions.
    Files(&'static [&'static str]),
}

/// One suggestion: the whole path to put in the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub path: String,
    pub folder: bool,
}

/// What `typed` could become. An empty field starts at `~/`; text without a
/// `/` is looked up in the home folder. Hidden entries show only once a `.`
/// is typed. Folders first, then files, each in name order.
pub fn complete(typed: &str, home: Option<&Path>, want: Want) -> Vec<Suggestion> {
    let typed = if typed.trim().is_empty() { "~/" } else { typed };
    let (dir, partial) = match typed.rfind('/') {
        Some(at) => typed.split_at(at + 1),
        None => ("~/", typed),
    };
    let Some(folder) = expand(dir, home) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Vec::new();
    };
    let partial = fold(partial);
    let mut found: Vec<Suggestion> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_lowercase();
            if !fold(&name).starts_with(&partial)
                || (name.starts_with('.') && !partial.starts_with('.'))
            {
                return None;
            }
            // Follows links, so a linked folder counts as a folder.
            let folder = entry.path().is_dir();
            let wanted = folder
                || match want {
                    Want::Folders => false,
                    Want::Files(extensions) => Path::new(&lower)
                        .extension()
                        .is_some_and(|e| extensions.contains(&e.to_string_lossy().as_ref())),
                };
            wanted.then(|| Suggestion {
                path: format!("{dir}{name}{}", if folder { "/" } else { "" }),
                folder,
            })
        })
        .collect();
    found.sort_by(|a, b| {
        b.folder
            .cmp(&a.folder)
            .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
    });
    found.truncate(MAX);
    found
}

/// The folder `dir` names: `~/` is home; anything else must be absolute.
fn expand(dir: &str, home: Option<&Path>) -> Option<PathBuf> {
    match dir.strip_prefix('~') {
        Some(rest) => Some(home?.join(rest.trim_start_matches('/'))),
        None => Some(PathBuf::from(dir)).filter(|path| path.is_absolute()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        for dir in ["Music", "Música vieja", "Videos", ".config", "bibles"] {
            std::fs::create_dir(home.path().join(dir)).unwrap();
        }
        std::fs::write(home.path().join("bibles/rv1909.tsv"), "").unwrap();
        std::fs::write(home.path().join("bibles/notes.txt"), "").unwrap();
        std::fs::write(home.path().join("mixtape.mp3"), "").unwrap();
        home
    }

    fn paths(found: Vec<Suggestion>) -> Vec<String> {
        found.into_iter().map(|s| s.path).collect()
    }

    #[test]
    fn an_empty_field_lists_the_home_folders() {
        let home = home();
        let found = complete("", Some(home.path()), Want::Folders);
        assert_eq!(
            paths(found),
            ["~/bibles/", "~/Music/", "~/Música vieja/", "~/Videos/"]
        );
    }

    #[test]
    fn typing_narrows_without_caring_about_case_and_hidden_needs_a_dot() {
        let home = home();
        assert_eq!(
            paths(complete("~/mu", Some(home.path()), Want::Folders)),
            ["~/Music/", "~/Música vieja/"]
        );
        assert_eq!(
            paths(complete("vid", Some(home.path()), Want::Folders)),
            ["~/Videos/"]
        );
        assert_eq!(
            paths(complete("~/musica", Some(home.path()), Want::Folders)),
            ["~/Música vieja/"],
            "accents don't matter"
        );
        assert_eq!(
            paths(complete("~/.c", Some(home.path()), Want::Folders)),
            ["~/.config/"]
        );
    }

    #[test]
    fn files_show_only_with_a_wanted_extension_and_absolute_paths_work() {
        let home = home();
        let bibles = Want::Files(&["tsv", "json"]);
        let found = complete("~/bibles/", Some(home.path()), bibles);
        assert_eq!(paths(found.clone()), ["~/bibles/rv1909.tsv"]);
        assert!(!found[0].folder);

        let absolute = format!("{}/Vi", home.path().display());
        let found = complete(&absolute, None, Want::Folders);
        assert_eq!(paths(found), [format!("{}/Videos/", home.path().display())]);
        assert!(complete("relative/", Some(home.path()), Want::Folders).is_empty());
    }
}
