//! A plan to take along: the plan and the media it shows, in one file,
//! to build at home and open on the computer at church.
//!
//! `Sábado.lantern` is a zip, with nothing compressed (pictures and
//! videos already are):
//!
//! ```text
//! plan.json     the plan, with the paths below written relative
//! files/        each file a step, the standby shelf or the look shows
//! folders/      the plan's own folders, whole
//! youtube/      its YouTube videos, as downloaded
//! ```
//!
//! [`export`] writes one; [`import`] copies its media into Lantern's data
//! folder and returns the plan with its paths pointing there, so the
//! file can go back in a pocket. A file that's missing at export is left
//! out and named in the report; nothing else stops for it.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use lantern_engine::{Action, Location, Plan, Resource, StepId};

use crate::plans::slug;

/// What a bundle's file name ends in.
pub const SUFFIX: &str = "lantern";
const PLAN: &str = "plan.json";
const FILES: &str = "files";
const FOLDERS: &str = "folders";
const YOUTUBE: &str = "youtube";

/// What an export did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    /// The bundle.
    pub dir: PathBuf,
    /// How many files it holds besides the plan, and their size.
    pub files: usize,
    pub bytes: u64,
    /// What the plan shows that wasn't there to copy, by name.
    pub missing: Vec<String>,
}

impl Exported {
    /// "Exported to ~/Documents/Sábado.lantern: 12 files, 340 MB", and
    /// what was left out.
    pub fn said(&self, home: Option<&Path>) -> String {
        let dir = match home {
            Some(home) => crate::config::tilde(&self.dir, home),
            None => self.dir.clone(),
        };
        let mut said = format!(
            "Exported to {}: {}, {}",
            dir.display(),
            match self.files {
                1 => "1 file".to_string(),
                n => format!("{n} files"),
            },
            size(self.bytes)
        );
        if !self.missing.is_empty() {
            said.push_str(&format!(". Not found: {}", self.missing.join(", ")));
        }
        said
    }
}

/// `340 MB`, `1.2 GB`, `12 KB`.
fn size(bytes: u64) -> String {
    const KB: u64 = 1024;
    match bytes {
        b if b >= KB * KB * KB => format!("{:.1} GB", b as f64 / (KB * KB * KB) as f64),
        b if b >= KB * KB => format!("{} MB", b / (KB * KB)),
        b => format!("{} KB", b.div_ceil(KB)),
    }
}

/// The files a plan shows, to rewrite each the same way: its steps', its
/// standby shelf's, and its look's image.
fn rewrite(plan: &mut Plan, mut place: impl FnMut(&Path, &str) -> Option<PathBuf>) {
    let mut resource = |resource: &mut Resource| {
        if let Location::File(path) = &resource.location
            && let Some(placed) = place(path, &resource.title)
        {
            resource.location = Location::File(placed);
        }
    };
    let ids: Vec<StepId> = plan.steps().iter().map(|step| step.id).collect();
    for id in ids {
        if let Some(Action::Show { resource: shown }) = plan.get_mut(id).map(|s| &mut s.action) {
            resource(shown);
        }
    }
    plan.standby.iter_mut().for_each(&mut resource);
    if let Some(look) = &mut plan.look
        && let Some(image) = &look.image
        && let Some(placed) = place(image, "the look's image")
    {
        look.image = Some(placed);
    }
}

/// Every file and folder `plan` needs from the disk.
pub fn media(plan: &Plan) -> Vec<PathBuf> {
    let mut paths = plan.folders.clone();
    let mut copy = plan.clone();
    rewrite(&mut copy, |path, _| {
        paths.push(path.to_path_buf());
        None
    });
    paths
}

/// A name that's safe as a file's on any system: the title without the
/// characters a path can't hold.
fn file_safe(title: &str) -> String {
    let name: String = title
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let name = name.trim_matches(['.', ' ']).to_string();
    if name.is_empty() { "plan".into() } else { name }
}

/// `name` in `dir`, or `2-name`, `3-name`… when it's taken.
fn free(dir: &Path, name: &str, taken: &mut Vec<String>) -> PathBuf {
    let pick = (1..)
        .map(|n| match n {
            1 => name.to_string(),
            n => format!("{n}-{name}"),
        })
        .find(|name| !taken.contains(name))
        .expect("some numbered name is free");
    taken.push(pick.clone());
    dir.join(pick)
}

/// The name of an entry: the path inside the bundle, with `/` between
/// its parts whatever the system.
fn entry_name(inside: &Path) -> String {
    let parts: Vec<_> = inside
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect();
    parts.join("/")
}

/// A bundle being written.
struct Pack {
    zip: zip::ZipWriter<File>,
    /// The entries so far.
    names: HashSet<String>,
    /// How many files went in besides the plan, and their size.
    files: usize,
    bytes: u64,
}

impl Pack {
    fn start(&mut self, name: &str, size: u64) -> io::Result<()> {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(size >= u64::from(u32::MAX));
        self.zip.start_file(name, options).map_err(io::Error::other)
    }

    /// Adds the file at `from` as `inside`.
    fn add(&mut self, from: &Path, inside: &Path) -> io::Result<()> {
        let mut file = File::open(from)?;
        let name = entry_name(inside);
        self.start(&name, file.metadata()?.len())?;
        self.bytes += io::copy(&mut file, &mut self.zip)?;
        self.files += 1;
        self.names.insert(name);
        Ok(())
    }

    /// Adds the folder at `from` and everything under it, as `inside`. A
    /// link to a file goes in as the file; a link to a folder is skipped,
    /// so a loop can't go on for ever.
    fn add_tree(&mut self, from: &Path, inside: &Path) -> io::Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(from)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let (source, target) = (entry.path(), inside.join(entry.file_name()));
            let kind = entry.file_type()?;
            if kind.is_dir() {
                self.add_tree(&source, &target)?;
            } else if kind.is_file() || source.is_file() {
                self.add(&source, &target)?;
            }
        }
        Ok(())
    }

    fn has(&self, inside: &Path) -> bool {
        self.names.contains(&entry_name(inside))
    }

    /// Adds the plan and closes the file.
    fn finish(mut self, plan: &Plan) -> io::Result<()> {
        let json = plan.to_json();
        self.start(PLAN, json.len() as u64)?;
        self.zip.write_all(json.as_bytes())?;
        self.zip.finish().map_err(io::Error::other)?.sync_all()
    }
}

/// Opens the bundle at `path`, if it is one: a zip with a plan in it.
fn open(path: &Path) -> Option<zip::ZipArchive<File>> {
    let mut zip = zip::ZipArchive::new(File::open(path).ok()?).ok()?;
    let has_plan = zip.by_name(PLAN).is_ok();
    has_plan.then_some(zip)
}

fn failed(what: &str, path: &Path, err: impl std::fmt::Display) -> String {
    format!("Couldn't {what} {}: {err}", path.display())
}

/// Writes `plan` and its media as `<title>.lantern` in `into`, replacing
/// an earlier export of the same name. `resolve` turns one of the plan's
/// own folders, as typed (`~/Church/Campaña`), into its path; `youtube`
/// are the downloaded files of its YouTube videos.
pub fn export(
    plan: &Plan,
    into: &Path,
    resolve: &dyn Fn(&Path) -> PathBuf,
    youtube: &[PathBuf],
) -> Result<Exported, String> {
    let name = format!("{}.{SUFFIX}", file_safe(&plan.title));
    let dir = into.join(&name);
    // Something else by that name stays where it is.
    if dir.exists() && open(&dir).is_none() {
        return Err(format!(
            "{} is in the way, and isn't an exported plan.",
            dir.display()
        ));
    }
    // Written beside it and swapped in, so half an export never looks whole.
    let partial = into.join(format!("{name}.partial"));
    let file = File::create(&partial).map_err(|err| failed("make", &partial, err))?;
    let mut pack = Pack {
        zip: zip::ZipWriter::new(file),
        names: HashSet::new(),
        files: 0,
        bytes: 0,
    };
    let packed = pack_plan(plan, &mut pack, resolve, youtube).and_then(|(out, missing)| {
        let (files, bytes) = (pack.files, pack.bytes);
        pack.finish(&out)
            .and_then(|()| std::fs::rename(&partial, &dir))
            .map_err(|err| failed("write", &dir, err))?;
        Ok(Exported {
            dir,
            files,
            bytes,
            missing,
        })
    });
    if packed.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    packed
}

/// Puts the media of `plan` into `pack`. Returns the plan as the bundle
/// holds it, its paths relative, and what wasn't there to add.
fn pack_plan(
    plan: &Plan,
    pack: &mut Pack,
    resolve: &dyn Fn(&Path) -> PathBuf,
    youtube: &[PathBuf],
) -> Result<(Plan, Vec<String>), String> {
    let mut out = plan.clone();
    let mut missing = Vec::new();

    // Its own folders, whole.
    let mut names = Vec::new();
    let mut copied: Vec<(PathBuf, PathBuf)> = Vec::new();
    out.folders.clear();
    for folder in &plan.folders {
        let source = resolve(folder);
        if !source.is_dir() {
            missing.push(folder.display().to_string());
            continue;
        }
        let name = source
            .file_name()
            .map_or("folder".into(), |name| name.to_string_lossy().into_owned());
        let inside = free(Path::new(FOLDERS), &name, &mut names);
        pack.add_tree(&source, &inside)
            .map_err(|err| failed("copy", &source, err))?;
        out.folders.push(inside.clone());
        copied.push((source, inside));
    }

    // Each file it shows: where its folder went, or else among the files.
    let mut names = Vec::new();
    let mut placed: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut error = None;
    rewrite(&mut out, |path, title| {
        if let Some(inside) = placed.get(path) {
            return Some(inside.clone());
        }
        let in_folder = copied
            .iter()
            .find_map(|(source, inside)| Some(inside.join(path.strip_prefix(source).ok()?)));
        let inside = match in_folder {
            Some(inside) if pack.has(&inside) => inside,
            _ if !path.is_file() => {
                missing.push(title.to_string());
                return None;
            }
            _ => {
                let name = path
                    .file_name()
                    .map_or("file".into(), |name| name.to_string_lossy().into_owned());
                let inside = free(Path::new(FILES), &name, &mut names);
                if let Err(err) = pack.add(path, &inside) {
                    error.get_or_insert(failed("copy", path, err));
                    return None;
                }
                inside
            }
        };
        placed.insert(path.to_path_buf(), inside.clone());
        Some(inside)
    });
    if let Some(error) = error {
        return Err(error);
    }

    // Its YouTube videos, so the other computer needn't download them.
    for video in youtube.iter().filter(|video| video.is_file()) {
        let Some(name) = video.file_name() else {
            continue;
        };
        pack.add(video, &Path::new(YOUTUBE).join(name))
            .map_err(|err| failed("copy", video, err))?;
    }
    missing.dedup();
    Ok((out, missing))
}

/// The bundles lying in `places`: where an export or a USB stick usually
/// is. Sorted by name.
pub fn find(places: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = places
        .iter()
        .filter_map(|place| std::fs::read_dir(place).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == SUFFIX))
        .filter(|path| path.is_file() && open(path).is_some())
        .collect();
    found.sort();
    found.dedup();
    found
}

/// Where bundles are looked for: Documents, Downloads, the desktop and
/// home, and whatever is plugged in.
pub fn places(home: Option<&Path>, user: Option<&str>) -> Vec<PathBuf> {
    let mut places = Vec::new();
    if let Some(home) = home {
        places.extend(["Documents", "Downloads", "Desktop", ""].map(|dir| home.join(dir)));
    }
    // Removable media: each mounted volume is a place.
    let mut mounts = vec![PathBuf::from("/Volumes"), PathBuf::from("/mnt")];
    if let Some(user) = user {
        mounts.push(Path::new("/run/media").join(user));
        mounts.push(Path::new("/media").join(user));
    }
    for mount in mounts {
        let volumes = std::fs::read_dir(&mount).into_iter().flatten().flatten();
        places.extend(
            volumes
                .map(|entry| entry.path())
                .filter(|path| path.is_dir()),
        );
    }
    places
}

/// Copies the media of the bundle at `bundle` into a folder of its own
/// under `media`, and its YouTube videos into the `youtube` cache, and
/// returns the plan with its paths pointing at the copies.
pub fn import(bundle: &Path, media: &Path, youtube: &Path) -> Result<Plan, String> {
    let mut zip =
        open(bundle).ok_or_else(|| format!("{} isn't an exported plan.", bundle.display()))?;
    let mut json = String::new();
    zip.by_name(PLAN)
        .map_err(io::Error::other)
        .and_then(|mut file| file.read_to_string(&mut json))
        .map_err(|err| failed("read", bundle, err))?;
    let mut plan = Plan::from_json(&json).map_err(|err| failed("read", bundle, err))?;
    let name = slug(&plan.title);
    let dir = (1..)
        .map(|n| match n {
            1 => media.join(&name),
            n => media.join(format!("{name}-{n}")),
        })
        .find(|dir| !dir.exists())
        .expect("some numbered name is free");
    if let Err(err) = unpack(&mut zip, &dir, youtube) {
        // Half its media is worse than none.
        let _ = std::fs::remove_dir_all(&dir);
        return Err(failed("copy", bundle, err));
    }
    // Only paths inside the bundle are its own.
    let inside = |path: &Path| {
        path.is_relative() && path.components().all(|c| matches!(c, Component::Normal(_)))
    };
    for folder in &mut plan.folders {
        if inside(folder) {
            *folder = dir.join(&*folder);
        }
    }
    rewrite(&mut plan, |path, _| inside(path).then(|| dir.join(path)));
    Ok(plan)
}

/// Writes the bundle's files and folders under `dir`, and its videos
/// into `youtube` unless they're there already. An entry whose name
/// would land outside is skipped.
fn unpack(zip: &mut zip::ZipArchive<File>, dir: &Path, youtube: &Path) -> io::Result<()> {
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(io::Error::other)?;
        let Some(name) = entry.enclosed_name().filter(|_| entry.is_file()) else {
            continue;
        };
        let target = match name.components().next() {
            Some(Component::Normal(part)) if part == FILES || part == FOLDERS => dir.join(&name),
            Some(Component::Normal(part)) if part == YOUTUBE => match name.file_name() {
                Some(video) if !youtube.join(video).exists() => youtube.join(video),
                _ => continue,
            },
            _ => continue,
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        io::copy(&mut entry, &mut File::create(&target)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use lantern_engine::{Kind, Look, ResourceId, SourceId};

    use super::*;

    fn file(path: &Path, kind: Kind) -> Resource {
        let source = SourceId::new("local").unwrap();
        let title = path.file_stem().unwrap().to_string_lossy().into_owned();
        Resource::new(
            ResourceId::new(&source, &path.to_string_lossy()),
            title,
            kind,
            Location::File(path.to_path_buf()),
        )
    }

    fn shown(plan: &Plan) -> Vec<PathBuf> {
        plan.steps()
            .iter()
            .filter_map(|step| match &step.action {
                Action::Show { resource } => match &resource.location {
                    Location::File(path) => Some(path.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    /// A home with a campaign folder, two loose pictures of the same name,
    /// a look's image, a downloaded video, and the plan that uses them.
    fn home() -> (tempfile::TempDir, Plan, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let at = |path: &str| home.path().join(path);
        for (path, text) in [
            ("Church/Campaña/lema.png", "lema"),
            ("Church/Campaña/dia 1/tema.pdf", "tema"),
            ("Pictures/aviso.png", "aviso"),
            ("Downloads/aviso.png", "otro aviso"),
            ("Pictures/fondo.jpg", "fondo"),
            ("cache/youtube/abc12345678.mp4", "video"),
        ] {
            std::fs::create_dir_all(at(path).parent().unwrap()).unwrap();
            std::fs::write(at(path), text).unwrap();
        }
        let mut plan = Plan::new("Sábado: Campaña");
        plan.folders = vec!["~/Church/Campaña".into()];
        plan.section("Apertura", None);
        plan.show(file(&at("Church/Campaña/dia 1/tema.pdf"), Kind::Document));
        plan.show(file(&at("Pictures/aviso.png"), Kind::Image));
        plan.show(file(&at("Pictures/aviso.png"), Kind::Image));
        plan.show(file(&at("Pictures/gone.png"), Kind::Image));
        plan.push("Oración", Action::Blank);
        plan.keep_ready(file(&at("Downloads/aviso.png"), Kind::Image));
        plan.look = Some(Look {
            image: Some(at("Pictures/fondo.jpg")),
            ..Look::default()
        });
        let video = at("cache/youtube/abc12345678.mp4");
        (home, plan, video)
    }

    /// The text of an entry of the bundle, if it has it.
    fn entry(bundle: &Path, name: &str) -> Option<String> {
        let mut zip = open(bundle)?;
        let mut text = String::new();
        zip.by_name(name).ok()?.read_to_string(&mut text).ok()?;
        Some(text)
    }

    #[test]
    fn a_plan_goes_out_with_its_media_and_comes_back_whole() {
        let (home, plan, video) = home();
        let resolve = |path: &Path| match path.strip_prefix("~") {
            Ok(rest) => home.path().join(rest),
            Err(_) => path.to_path_buf(),
        };
        let out = home.path().join("Documents");
        std::fs::create_dir_all(&out).unwrap();
        let exported = export(&plan, &out, &resolve, std::slice::from_ref(&video)).unwrap();
        let bundle = out.join("Sábado- Campaña.lantern");
        assert_eq!(exported.dir, bundle);
        assert!(bundle.is_file(), "one file");
        // The folder's two files, two pictures, the look's image, the video.
        assert_eq!(exported.files, 6);
        assert_eq!(exported.missing, ["gone"]);
        assert!(exported.said(Some(home.path())).starts_with(
            "Exported to ~/Documents/Sábado- Campaña.lantern: 6 files, 1 KB. Not found: gone"
        ));
        let inside = Plan::from_json(&entry(&bundle, PLAN).unwrap()).unwrap();
        assert_eq!(inside.folders, [PathBuf::from("folders/Campaña")]);
        assert_eq!(
            shown(&inside),
            [
                // In its folder, where the folder went; a file shown twice
                // is there once; what's missing is left as it was.
                PathBuf::from("folders/Campaña/dia 1/tema.pdf"),
                PathBuf::from("files/aviso.png"),
                PathBuf::from("files/aviso.png"),
                home.path().join("Pictures/gone.png"),
            ]
        );
        // Another file of the same name gets its own.
        let on_standby = match &inside.standby[0].location {
            Location::File(path) => path.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(on_standby, PathBuf::from("files/2-aviso.png"));
        assert_eq!(
            entry(&bundle, "files/2-aviso.png").as_deref(),
            Some("otro aviso")
        );
        assert_eq!(
            entry(&bundle, "youtube/abc12345678.mp4").as_deref(),
            Some("video")
        );
        assert!(!out.join("Sábado- Campaña.lantern.partial").exists());

        // Exported again, it replaces the first.
        export(&plan, &out, &resolve, &[]).unwrap();
        assert_eq!(entry(&bundle, "youtube/abc12345678.mp4"), None);
        export(&plan, &out, &resolve, std::slice::from_ref(&video)).unwrap();

        // On the other computer: nothing of the first one's home is needed.
        let there = tempfile::tempdir().unwrap();
        let (media, cache) = (there.path().join("bundles"), there.path().join("youtube"));
        assert_eq!(
            find(std::slice::from_ref(&out)),
            std::slice::from_ref(&bundle)
        );
        let back = import(&bundle, &media, &cache).unwrap();
        let dir = media.join("sábado-campaña");
        assert_eq!(back.folders, [dir.join("folders/Campaña")]);
        assert_eq!(shown(&back)[0], dir.join("folders/Campaña/dia 1/tema.pdf"));
        assert_eq!(shown(&back)[1], dir.join("files/aviso.png"));
        for path in super::media(&back)
            .iter()
            .filter(|path| path.starts_with(&dir))
        {
            assert!(path.exists(), "{}", path.display());
        }
        assert_eq!(
            std::fs::read_to_string(dir.join("folders/Campaña/lema.png")).unwrap(),
            "lema"
        );
        assert_eq!(
            back.look.as_ref().and_then(|look| look.image.clone()),
            Some(dir.join("files/fondo.jpg"))
        );
        assert_eq!(
            std::fs::read_to_string(cache.join("abc12345678.mp4")).unwrap(),
            "video"
        );
        assert_eq!(back.title, plan.title);
        assert_eq!(back.len(), plan.len());
        // The same bundle again gets a folder of its own.
        let again = import(&bundle, &media, &cache).unwrap();
        assert_eq!(
            again.folders,
            [media.join("sábado-campaña-2/folders/Campaña")]
        );
    }

    /// A zip at `path` with these entries.
    fn zipped(path: &Path, entries: &[(&str, &str)]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, text) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn what_isnt_a_bundle_is_left_alone() {
        let home = tempfile::tempdir().unwrap();
        let plan = Plan::new("Notes");
        // A file of the user's own by the bundle's name is in the way.
        let theirs = home.path().join("Notes.lantern");
        std::fs::write(&theirs, "mine").unwrap();
        let err = export(&plan, home.path(), &|p| p.to_path_buf(), &[]).unwrap_err();
        assert!(err.contains("in the way"), "{err}");
        assert_eq!(std::fs::read_to_string(&theirs).unwrap(), "mine");
        assert!(!home.path().join("Notes.lantern.partial").exists());
        assert!(find(&[home.path().to_path_buf()]).is_empty());
        let err = import(&theirs, home.path(), home.path()).unwrap_err();
        assert!(err.contains("isn't an exported plan"), "{err}");
        // So is a zip without a plan in it.
        zipped(&theirs, &[("files/a.png", "a")]);
        assert!(import(&theirs, home.path(), home.path()).is_err());
        // A path that climbs out of the bundle is not taken for its own,
        // and an entry that would land outside isn't written.
        zipped(
            &theirs,
            &[
                (
                    PLAN,
                    r#"{"title":"X","steps":[],"folders":["../../etc","folders/ok"]}"#,
                ),
                ("folders/ok/a.png", "a"),
                ("../escaped.txt", "out"),
                ("files/../../escaped.txt", "out"),
                ("other/b.png", "b"),
            ],
        );
        let media = home.path().join("m");
        let plan = import(&theirs, &media, &home.path().join("yt")).unwrap();
        assert_eq!(
            plan.folders,
            [PathBuf::from("../../etc"), media.join("x/folders/ok")]
        );
        assert!(media.join("x/folders/ok/a.png").is_file());
        assert!(!home.path().join("escaped.txt").exists());
        assert!(!media.join("escaped.txt").exists());
        assert!(!media.join("x/other").exists());
        assert_eq!(file_safe(" a/b:c. "), "a-b-c");
        assert_eq!(file_safe("..."), "plan");
        assert_eq!(size(340 * 1024 * 1024), "340 MB");
        assert_eq!(size(1_300_000_000), "1.2 GB");
    }
}
