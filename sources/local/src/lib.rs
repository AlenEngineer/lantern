//! Local folders as a lantern [`Source`].
//!
//! Point it at the folders you keep media in (`Music`, `Videos`,
//! `Presentations`...) and it lists every file it recognizes, so the engine
//! can index them in memory. Searching is then instant and offline.
//!
//! Each file becomes a [`Resource`] with:
//!
//! - **title**: the file name without extension, `_` turned into spaces;
//! - **subtitle**: the folder label and the sub-folders, e.g. `Music / Hymns`;
//! - **tags**: the same folder names, so searching `hymns` finds everything inside;
//! - **meta**: `extension` and `size` (bytes).
//!
//! A folder whose files are named by number or code (`004.mp4`) can have a
//! names file, which gives each a title to show and to search by: see
//! [`Folder::named_by`].
//!
//! Hidden files and folders (starting with `.`) are skipped unless enabled.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::iter;
use std::path::{Path, PathBuf};

use lantern_resource::{
    Capabilities, Kind, Location, Resource, ResourceId, Source, SourceError, SourceId,
};
use walkdir::{DirEntry, WalkDir};

/// A folder to index.
#[derive(Debug, Clone)]
pub struct Folder {
    /// Where it is.
    pub path: PathBuf,
    /// Display name. Defaults to the folder's own name.
    pub label: Option<String>,
    /// Whether to descend into sub-folders (default: yes).
    pub recursive: bool,
    /// Only index these kinds. Empty means every kind recognized.
    pub kinds: Vec<Kind>,
    /// A file with titles for the folder's files: see [`Folder::named_by`].
    pub names: Option<PathBuf>,
}

impl Folder {
    /// A recursive folder, all kinds.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            label: None,
            recursive: true,
            kinds: Vec::new(),
            names: None,
        }
    }

    /// Takes the titles of the folder's files from a names file: text with
    /// a file and its title on each line, a tab between them.
    ///
    /// ```text
    /// # Lines starting with # are skipped.
    /// 004.mp4<tab>Himno 4 · Alabanzas sin cesar
    /// 005<tab>Himno 5 · A ti, glorioso Dios
    /// ```
    ///
    /// The file is named as it is in the folder, with its extension or
    /// without it (then every `005.*` takes the title), or by its path
    /// from the folder for one in a sub-folder (`coros/005.mp4`). A file
    /// the list leaves out keeps its own name, and so does every file if
    /// the list can't be read.
    pub fn named_by(mut self, names: impl Into<PathBuf>) -> Self {
        self.names = Some(names.into());
        self
    }

    /// Sets the display name.
    pub fn labeled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Don't descend into sub-folders.
    pub fn flat(mut self) -> Self {
        self.recursive = false;
        self
    }

    /// Only index `kind` (can be called several times).
    pub fn only(mut self, kind: Kind) -> Self {
        self.kinds.push(kind);
        self
    }

    fn display_label(&self) -> String {
        self.label.clone().unwrap_or_else(|| {
            self.path.file_name().map_or_else(
                || self.path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
        })
    }
}

/// Indexes files in a set of local folders.
#[derive(Debug, Clone)]
pub struct LocalSource {
    id: SourceId,
    name: String,
    folders: Vec<Folder>,
    include_hidden: bool,
    /// Its own bang, besides the `!f` every local source answers.
    bang: Option<String>,
}

impl LocalSource {
    /// A source with no folders yet.
    pub fn new(id: SourceId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            folders: Vec::new(),
            include_hidden: false,
            bang: None,
        }
    }

    /// Also answers `!bang`, besides `!f`.
    pub fn with_bang(mut self, bang: impl Into<String>) -> Self {
        self.bang = Some(bang.into());
        self
    }

    /// Adds a folder.
    pub fn with_folder(mut self, folder: Folder) -> Self {
        self.folders.push(folder);
        self
    }

    /// Whether to index hidden files and folders.
    pub fn include_hidden(mut self, include: bool) -> Self {
        self.include_hidden = include;
        self
    }

    /// The configured folders.
    pub fn folders(&self) -> &[Folder] {
        &self.folders
    }

    fn scan_folder(&self, folder: &Folder, out: &mut Vec<Resource>) {
        let depth = if folder.recursive { usize::MAX } else { 1 };
        let walker = WalkDir::new(&folder.path)
            .follow_links(true)
            .max_depth(depth)
            .into_iter()
            .filter_entry(|e| self.include_hidden || e.depth() == 0 || !is_hidden(e.file_name()));
        let names = folder
            .names
            .as_deref()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .map(|text| names(&text))
            .unwrap_or_default();
        // Unreadable entries are skipped: one bad file must not hide the rest.
        for entry in walker.filter_map(Result::ok) {
            if entry.file_type().is_file() {
                out.extend(self.resource_for(folder, &names, &entry));
            }
        }
    }

    fn resource_for(
        &self,
        folder: &Folder,
        names: &HashMap<String, String>,
        entry: &DirEntry,
    ) -> Option<Resource> {
        let path = entry.path();
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        let kind = Kind::from_extension(&extension)?;
        if !folder.kinds.is_empty() && !folder.kinds.contains(&kind) {
            return None;
        }

        let label = folder.display_label();
        let parents = relative_parents(&folder.path, path);
        let subtitle = iter::once(label.as_str())
            .chain(parents.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" / ");

        let mut resource = Resource::new(
            ResourceId::new(&self.id, &path.to_string_lossy()),
            title(names, &folder.path, path)?,
            kind,
            Location::File(path.to_path_buf()),
        )
        .with_subtitle(subtitle)
        .with_tag(label)
        .with_meta("extension", extension);
        for parent in parents {
            resource = resource.with_tag(parent);
        }
        if let Ok(metadata) = entry.metadata() {
            resource = resource.with_meta("size", metadata.len().to_string());
        }
        Some(resource)
    }
}

impl Source for LocalSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::INDEXED
    }

    fn bangs(&self) -> Vec<&str> {
        std::iter::once("f").chain(self.bang.as_deref()).collect()
    }

    /// Lists every recognized file. Folders that don't exist (an unplugged
    /// drive) are skipped; it's only an error if *none* of them exist.
    fn scan(&self) -> Result<Vec<Resource>, SourceError> {
        let present: Vec<&Folder> = self.folders.iter().filter(|f| f.path.is_dir()).collect();
        if present.is_empty() && !self.folders.is_empty() {
            return Err(SourceError::Unavailable(
                "none of the configured folders exist".into(),
            ));
        }
        let mut out = Vec::new();
        for folder in present {
            self.scan_folder(folder, &mut out);
        }
        Ok(out)
    }
}

fn is_hidden(name: &OsStr) -> bool {
    name.to_str().is_some_and(|s| s.starts_with('.'))
}

/// Folder names between `root` and the file.
fn relative_parents(root: &Path, file: &Path) -> Vec<String> {
    file.parent()
        .and_then(|p| p.strip_prefix(root).ok())
        .map(|rel| {
            rel.iter()
                .map(|c| c.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// What a names file says, by the file each line names.
fn names(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| line.split_once('\t'))
        .map(|(file, title)| (file.trim().to_owned(), title.trim().to_owned()))
        .filter(|(file, title)| !file.is_empty() && !title.is_empty())
        .collect()
}

/// A file's title: what `names` gives it, by its path from `root`, its
/// name or its name without extension, or else its own name made readable.
fn title(names: &HashMap<String, String>, root: &Path, file: &Path) -> Option<String> {
    let stem = file.file_stem()?.to_string_lossy();
    let from_root = file.strip_prefix(root).ok().map(|path| {
        path.iter()
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    });
    let name = file.file_name().map(|name| name.to_string_lossy());
    let named = [from_root.as_deref(), name.as_deref(), Some(&*stem)]
        .into_iter()
        .flatten()
        .find_map(|key| names.get(key));
    Some(named.cloned().unwrap_or_else(|| humanize(&stem)))
}

/// `"Santo_santo  santo"` → `"Santo santo santo"`.
fn humanize(stem: &str) -> String {
    stem.replace('_', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for file in [
            "Music/Hymns/Santo_santo_santo.mp3",
            "Music/intro.MP3",
            "Music/cover.jpg",
            "Music/.hidden/secret.mp3",
            "Music/notes.xyz",
            "Videos/Bienvenida.mp4",
        ] {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"data").unwrap();
        }
        dir
    }

    fn source(folders: impl IntoIterator<Item = Folder>) -> LocalSource {
        folders.into_iter().fold(
            LocalSource::new(SourceId::new("local").unwrap(), "Local"),
            LocalSource::with_folder,
        )
    }

    fn titles(mut items: Vec<Resource>) -> Vec<String> {
        items.sort_by(|a, b| a.title.cmp(&b.title));
        items.into_iter().map(|r| r.title).collect()
    }

    #[test]
    fn indexes_recognized_files_recursively() {
        let dir = tree();
        let items = source([Folder::new(dir.path().join("Music"))])
            .scan()
            .unwrap();
        assert_eq!(
            titles(items.clone()),
            ["Santo santo santo", "cover", "intro"]
        );

        let hymn = items
            .iter()
            .find(|r| r.title == "Santo santo santo")
            .unwrap();
        assert_eq!(hymn.kind, Kind::Audio);
        assert_eq!(hymn.subtitle.as_deref(), Some("Music / Hymns"));
        assert_eq!(hymn.tags, ["Music", "Hymns"]);
        assert_eq!(hymn.meta["extension"], "mp3");
        assert_eq!(hymn.meta["size"], "4");
        assert!(hymn.id.belongs_to(&SourceId::new("local").unwrap()));
    }

    #[test]
    fn flat_labeled_and_filtered_folders() {
        let dir = tree();
        let folder = Folder::new(dir.path().join("Music"))
            .flat()
            .labeled("Música")
            .only(Kind::Audio);
        let items = source([folder]).scan().unwrap();
        assert_eq!(titles(items.clone()), ["intro"]);
        assert_eq!(items[0].subtitle.as_deref(), Some("Música"));
    }

    #[test]
    fn a_names_file_gives_titles() {
        let dir = tree();
        let list = dir.path().join("names.tsv");
        fs::write(
            &list,
            "# our music\n\
             intro\tPreludio\n\
             cover.jpg\tPortada del disco\n\
             Hymns/Santo_santo_santo.mp3\t Himno 1 · Santo, santo, santo \n\
             not a line\n\
             nobody.mp3\tNadie\n",
        )
        .unwrap();
        let folder = Folder::new(dir.path().join("Music")).named_by(&list);
        let items = source([folder]).scan().unwrap();
        assert_eq!(
            titles(items),
            [
                "Himno 1 · Santo, santo, santo",
                "Portada del disco",
                "Preludio"
            ]
        );

        // A list that isn't there changes nothing.
        let folder = Folder::new(dir.path().join("Music")).named_by(dir.path().join("gone.tsv"));
        let items = source([folder]).scan().unwrap();
        assert_eq!(titles(items), ["Santo santo santo", "cover", "intro"]);
    }

    #[test]
    fn hidden_files_can_be_included() {
        let dir = tree();
        let items = source([Folder::new(dir.path().join("Music")).only(Kind::Audio)])
            .include_hidden(true)
            .scan()
            .unwrap();
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn missing_folders_are_skipped_unless_all_are_missing() {
        let dir = tree();
        let partial = source([
            Folder::new(dir.path().join("Videos")),
            Folder::new("/nope/lantern"),
        ]);
        assert_eq!(partial.scan().unwrap().len(), 1);

        let none = source([Folder::new("/nope/lantern")]);
        assert!(matches!(none.scan(), Err(SourceError::Unavailable(_))));
    }
}
