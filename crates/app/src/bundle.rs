//! A plan to take along: the plan and the media it shows, in one folder,
//! to build at home and open on the computer at church.
//!
//! ```text
//! Sábado.lantern/
//!   plan.json     the plan, with the paths below written relative
//!   files/        each file a step, the standby shelf or the look shows
//!   folders/      the plan's own folders, whole
//!   youtube/      its YouTube videos, as downloaded
//! ```
//!
//! [`export`] writes one; [`import`] copies its media into Lantern's data
//! folder and returns the plan with its paths pointing there, so the
//! bundle can go back in a pocket. A file that's missing at export is left
//! out and named in the report; nothing else stops for it.

use std::collections::HashMap;
use std::io;
use std::path::{Component, Path, PathBuf};

use lantern_engine::{Action, Location, Plan, Resource, StepId};

use crate::plans::slug;

/// What a bundle's folder name ends in.
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

/// Copies a file, making the folders above it. Returns its size.
fn copy_file(from: &Path, to: &Path) -> io::Result<u64> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::copy(from, to)
}

/// Copies a folder and everything under it: (files, bytes). A link to a
/// file is copied as the file; a link to a folder is skipped, so a loop
/// can't go on for ever.
fn copy_tree(from: &Path, to: &Path) -> io::Result<(usize, u64)> {
    std::fs::create_dir_all(to)?;
    let (mut files, mut bytes) = (0, 0);
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type()?;
        if kind.is_dir() {
            let (f, b) = copy_tree(&source, &target)?;
            files += f;
            bytes += b;
        } else if kind.is_file() || source.is_file() {
            bytes += std::fs::copy(&source, &target)?;
            files += 1;
        }
    }
    Ok((files, bytes))
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
    if dir.exists() && !dir.join(PLAN).is_file() {
        return Err(format!(
            "{} is in the way, and isn't an exported plan.",
            dir.display()
        ));
    }
    // Built beside it and swapped in, so half an export never looks whole.
    let partial = into.join(format!("{name}.partial"));
    if partial.exists() {
        std::fs::remove_dir_all(&partial).map_err(|err| failed("clear", &partial, err))?;
    }
    std::fs::create_dir_all(&partial).map_err(|err| failed("make", &partial, err))?;

    let mut out = plan.clone();
    let (mut files, mut bytes) = (0, 0);
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
        let (f, b) = copy_tree(&source, &partial.join(&inside))
            .map_err(|err| failed("copy", &source, err))?;
        files += f;
        bytes += b;
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
            Some(inside) if partial.join(&inside).is_file() => inside,
            _ if !path.is_file() => {
                missing.push(title.to_string());
                return None;
            }
            _ => {
                let name = path
                    .file_name()
                    .map_or("file".into(), |name| name.to_string_lossy().into_owned());
                let inside = free(Path::new(FILES), &name, &mut names);
                match copy_file(path, &partial.join(&inside)) {
                    Ok(size) => {
                        files += 1;
                        bytes += size;
                    }
                    Err(err) => {
                        error.get_or_insert(failed("copy", path, err));
                        return None;
                    }
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
        bytes += copy_file(video, &partial.join(YOUTUBE).join(name))
            .map_err(|err| failed("copy", video, err))?;
        files += 1;
    }

    let file = partial.join(PLAN);
    std::fs::write(&file, out.to_json()).map_err(|err| failed("write", &file, err))?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|err| failed("replace", &dir, err))?;
    }
    std::fs::rename(&partial, &dir).map_err(|err| failed("finish", &dir, err))?;
    missing.dedup();
    Ok(Exported {
        dir,
        files,
        bytes,
        missing,
    })
}

/// The folder of a bundle at `path`: itself, or the folder of the
/// `plan.json` it names. `None` if it isn't one.
pub fn bundle_at(path: &Path) -> Option<PathBuf> {
    if path.join(PLAN).is_file() {
        Some(path.to_path_buf())
    } else if path.file_name().is_some_and(|name| name == PLAN) && path.is_file() {
        path.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

/// The bundles lying in `places`, one level down: where an export or a
/// USB stick usually is. Sorted by name.
pub fn find(places: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = places
        .iter()
        .filter_map(|place| std::fs::read_dir(place).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == SUFFIX))
        .filter(|path| path.join(PLAN).is_file())
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
    let bundle =
        bundle_at(bundle).ok_or_else(|| format!("{} isn't an exported plan.", bundle.display()))?;
    let file = bundle.join(PLAN);
    let mut plan = Plan::load(&file).map_err(|err| failed("read", &file, err))?;
    // Only paths inside the bundle are its own.
    let inside = |path: &Path| {
        path.is_relative() && path.components().all(|c| matches!(c, Component::Normal(_)))
    };
    let name = slug(&plan.title);
    let dir = (1..)
        .map(|n| match n {
            1 => media.join(&name),
            n => media.join(format!("{name}-{n}")),
        })
        .find(|dir| !dir.exists())
        .expect("some numbered name is free");
    for part in [FILES, FOLDERS] {
        let source = bundle.join(part);
        if source.is_dir() {
            copy_tree(&source, &dir.join(part)).map_err(|err| failed("copy", &source, err))?;
        }
    }
    for folder in &mut plan.folders {
        if inside(folder) {
            *folder = dir.join(&*folder);
        }
    }
    rewrite(&mut plan, |path, _| inside(path).then(|| dir.join(path)));
    // A video already downloaded here stays as it is.
    let videos = std::fs::read_dir(bundle.join(YOUTUBE))
        .into_iter()
        .flatten()
        .flatten();
    for video in videos
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
    {
        let Some(name) = video.file_name() else {
            continue;
        };
        let cached = youtube.join(name);
        if !cached.exists() {
            copy_file(&video, &cached).map_err(|err| failed("copy", &video, err))?;
        }
    }
    Ok(plan)
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
        // The folder's two files, two pictures, the look's image, the video.
        assert_eq!(exported.files, 6);
        assert_eq!(exported.missing, ["gone"]);
        assert!(exported.said(Some(home.path())).starts_with(
            "Exported to ~/Documents/Sábado- Campaña.lantern: 6 files, 1 KB. Not found: gone"
        ));
        let inside = Plan::load(bundle.join(PLAN)).unwrap();
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
            std::fs::read_to_string(bundle.join(&on_standby)).unwrap(),
            "otro aviso"
        );
        assert!(bundle.join("youtube/abc12345678.mp4").is_file());
        assert!(!out.join("Sábado- Campaña.lantern.partial").exists());

        // Exported again, it replaces the first.
        std::fs::write(bundle.join("stale"), "").unwrap();
        export(&plan, &out, &resolve, &[]).unwrap();
        assert!(!bundle.join("stale").exists());
        assert!(!bundle.join("youtube").exists());
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
        for path in media_of(&back).iter().filter(|path| path.starts_with(&dir)) {
            assert!(path.exists(), "{}", path.display());
        }
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
        let again = import(&bundle.join(PLAN), &media, &cache).unwrap();
        assert_eq!(
            again.folders,
            [media.join("sábado-campaña-2/folders/Campaña")]
        );
    }

    fn media_of(plan: &Plan) -> Vec<PathBuf> {
        media(plan)
    }

    #[test]
    fn what_isnt_a_bundle_is_left_alone() {
        let home = tempfile::tempdir().unwrap();
        let plan = Plan::new("Notes");
        // A folder of the user's own by the bundle's name is in the way.
        let theirs = home.path().join("Notes.lantern");
        std::fs::create_dir_all(&theirs).unwrap();
        std::fs::write(theirs.join("diary.txt"), "mine").unwrap();
        let err = export(&plan, home.path(), &|p| p.to_path_buf(), &[]).unwrap_err();
        assert!(err.contains("in the way"), "{err}");
        assert!(theirs.join("diary.txt").is_file());
        assert!(find(&[home.path().to_path_buf()]).is_empty());
        let err = import(&theirs, home.path(), home.path()).unwrap_err();
        assert!(err.contains("isn't an exported plan"), "{err}");
        // A path that climbs out of the bundle is not taken for its own.
        std::fs::write(
            theirs.join(PLAN),
            r#"{"title":"X","steps":[],"folders":["../../etc","folders/ok"]}"#,
        )
        .unwrap();
        let plan = import(&theirs, &home.path().join("m"), home.path()).unwrap();
        assert_eq!(
            plan.folders,
            [
                PathBuf::from("../../etc"),
                home.path().join("m/x/folders/ok")
            ]
        );
        assert_eq!(file_safe(" a/b:c. "), "a-b-c");
        assert_eq!(file_safe("..."), "plan");
        assert_eq!(size(340 * 1024 * 1024), "340 MB");
        assert_eq!(size(1_300_000_000), "1.2 GB");
    }
}
