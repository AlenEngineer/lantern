//! Slide decks: PowerPoint files and their like.
//!
//! The stage shows a deck as pages. LibreOffice turns it into a PDF, once,
//! kept in the cache, and the PDF is rendered like any other. What moves
//! in a deck (animations, transitions) is lost: each slide is a picture
//! of how it ends.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lantern_engine::{Kind, Location, Resource, ResourceId, SourceId};
use lantern_source_link::LinkSource;

/// The files LibreOffice is asked to turn into slides, by extension.
const EXTENSIONS: [&str; 5] = ["pptx", "ppt", "ppsx", "pps", "odp"];

/// Where LibreOffice's program is looked for, in order: on the `PATH`,
/// then where its installers put it.
const PROGRAMS: [&str; 4] = [
    "soffice",
    "libreoffice",
    "/Applications/LibreOffice.app/Contents/MacOS/soffice",
    r"C:\Program Files\LibreOffice\program\soffice.exe",
];

/// The longest a deck may take to convert. LibreOffice has been seen to
/// wait forever on a file it can't read.
const PATIENCE: Duration = Duration::from_secs(180);

/// One conversion at a time: two LibreOffices sharing a profile, the
/// second leaves at once having done nothing.
static CONVERTING: Mutex<()> = Mutex::new(());

/// Whether `path` is a deck the stage can show as slides.
pub fn is_deck(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXTENSIONS.iter().any(|deck| ext.eq_ignore_ascii_case(deck)))
}

/// The deck at `path` as a PDF in a folder under `cache`, keyed by the
/// file's path, size and modification time, made the first time and
/// reused after. Blocks while LibreOffice works: seconds.
pub fn to_pdf(path: &Path, cache: &Path) -> Result<PathBuf, String> {
    let fail = |err: &dyn std::fmt::Display| format!("{}: {err}", path.display());
    let dir = folder(path, cache).map_err(|err| fail(&err))?;
    let pdf = dir.join("deck.pdf");
    let _one = CONVERTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if pdf.is_file() {
        return Ok(pdf);
    }
    std::fs::create_dir_all(&dir).map_err(|err| fail(&err))?;
    // Its own profile, so it never meets a LibreOffice the operator has
    // open, which would take the file and convert nothing.
    let profile = url::Url::from_file_path(cache.join("profile"))
        .map_err(|()| fail(&"the cache isn't an absolute path"))?;
    let mut office = office(|program| {
        Command::new(program)
            .arg(format!("-env:UserInstallation={profile}"))
            .args([
                "--headless",
                "--norestore",
                "--convert-to",
                "pdf",
                "--outdir",
            ])
            .arg(&dir)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
    })
    .map_err(|err| fail(&err))?;
    let started = Instant::now();
    let status = loop {
        match office.try_wait().map_err(|err| fail(&err))? {
            Some(status) => break status,
            None if started.elapsed() > PATIENCE => {
                let _ = office.kill();
                let _ = office.wait();
                return Err(fail(&"LibreOffice took too long to read it"));
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    // It names the PDF after the deck. It also leaves with success when
    // it couldn't read the file, having written nothing.
    let made = std::fs::read_dir(&dir)
        .map_err(|err| fail(&err))?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .find(|file| file.extension().is_some_and(|ext| ext == "pdf"));
    let Some(made) = made else {
        let mut said = String::new();
        if let Some(mut stderr) = office.stderr.take() {
            let _ = stderr.read_to_string(&mut said);
        }
        let said = said.trim();
        return Err(fail(&match (status.success(), said.is_empty()) {
            (_, false) => format!("LibreOffice couldn't read it: {said}"),
            (true, true) => "LibreOffice couldn't read it".to_string(),
            (false, true) => format!("LibreOffice couldn't read it ({status})"),
        }));
    };
    std::fs::rename(&made, &pdf).map_err(|err| fail(&err))?;
    Ok(pdf)
}

/// The folder under `cache` for what is made of the deck at `path`, named
/// by the file's path, size and modification time: a deck that changes
/// gets another.
fn folder(path: &Path, cache: &Path) -> std::io::Result<PathBuf> {
    let meta = std::fs::metadata(path)?;
    let mut key = DefaultHasher::new();
    (path, meta.len(), meta.modified().ok()).hash(&mut key);
    Ok(cache.join(format!("{:016x}", key.finish())))
}

/// Starts LibreOffice with `start`, trying each place it may be.
fn office(
    start: impl Fn(&str) -> std::io::Result<std::process::Child>,
) -> Result<std::process::Child, String> {
    for program in PROGRAMS {
        match start(program) {
            Ok(child) => return Ok(child),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(format!("{program}: {err}")),
        }
    }
    Err(NO_OFFICE.to_string())
}

/// What a deck says when LibreOffice isn't there.
pub const NO_OFFICE: &str = "showing a PowerPoint file needs LibreOffice, and it isn't installed";

/// The small picture a deck carries of its first slide, as PowerPoint
/// and LibreOffice both save one: the file's bytes, a JPEG or PNG.
pub fn thumbnail(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|err| err.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|err| err.to_string())?;
    // PowerPoint's, then LibreOffice's.
    for name in ["docProps/thumbnail.jpeg", "Thumbnails/thumbnail.png"] {
        if let Ok(mut entry) = zip.by_name(name) {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|err| err.to_string())?;
            return Ok(bytes);
        }
    }
    Err("the deck has no picture of itself".to_string())
}

/// What each slide of the deck `deck` plays or links to that the stage can
/// show, as resources: a list for each slide. A video inside the deck is a
/// file under `cache`, where [`take_out`] puts it; a YouTube link is the
/// video; a link to a picture is the picture. Sounds and plain web pages
/// are left out: the stage has no way to show them.
pub fn attached(deck: &Resource, cache: &Path) -> Vec<Vec<Arc<Resource>>> {
    let Location::File(path) = &deck.location else {
        return Vec::new();
    };
    let source = SourceId::new("deck").expect("a valid source id");
    let links = LinkSource::new(SourceId::new("link").expect("a valid source id"));
    attachments(path)
        .into_iter()
        .enumerate()
        .map(|(slide, found)| {
            found
                .into_iter()
                .filter_map(|attachment| match attachment {
                    Attachment::Inside(entry) => {
                        let file = inside_path(path, &entry, cache)?;
                        let kind = Kind::from_extension(
                            &file.extension()?.to_str()?.to_ascii_lowercase(),
                        )?;
                        Some(
                            Resource::new(
                                ResourceId::new(&source, &file.to_string_lossy()),
                                format!("{} · slide {}", deck.title, slide + 1),
                                kind,
                                Location::File(file),
                            )
                            .with_subtitle("In the deck")
                            .with_meta(ENTRY, entry),
                        )
                    }
                    Attachment::Link(link) => links.resolve(&link),
                })
                .filter(|resource| crate::stage::needs(resource).is_ok())
                .map(Arc::new)
                .collect()
        })
        .collect()
}

/// The meta key of a resource from inside a deck: its name in the zip.
pub const ENTRY: &str = "deck_entry";

/// Something a slide plays or points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// A video or sound kept inside the deck: its name in the zip, like
    /// `ppt/media/media1.mp4`.
    Inside(String),
    /// A link: an online video, or one on some text or picture.
    Link(String),
}

/// What each slide of the PowerPoint file at `path` plays or links to, a
/// list for each slide in the order the slides are shown. Hidden slides
/// are left out, as they are from the pages. Empty for a file that isn't
/// one or can't be read: a deck shows without its attachments.
///
/// A `.pptx` is a zip. `ppt/presentation.xml` lists the slides in order,
/// each by a relationship id that `ppt/_rels/presentation.xml.rels` gives
/// a file to, and beside every slide `_rels/slideN.xml.rels` lists what it
/// points to: a video or sound inside the zip, or a link outside it.
pub fn attachments(path: &Path) -> Vec<Vec<Attachment>> {
    read_attachments(path).unwrap_or_default()
}

fn read_attachments(path: &Path) -> Option<Vec<Vec<Attachment>>> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let mut text = |name: &str| {
        let mut text = String::new();
        zip.by_name(name).ok()?.read_to_string(&mut text).ok()?;
        Some(text)
    };
    let files: std::collections::HashMap<String, String> =
        relationships(&text("ppt/_rels/presentation.xml.rels")?)
            .into_iter()
            .map(|rel| (rel.id, join("ppt", &rel.target)))
            .collect();
    let order = slide_ids(&text("ppt/presentation.xml")?);
    let mut slides = Vec::new();
    for id in order {
        let Some(slide) = files.get(&id) else {
            continue;
        };
        if text(slide).is_some_and(|xml| hidden(&xml)) {
            continue;
        }
        let (folder, name) = slide.rsplit_once('/').unwrap_or(("", slide));
        let mut found: Vec<Attachment> = Vec::new();
        let rels = text(&format!("{folder}/_rels/{name}.rels")).unwrap_or_default();
        for rel in relationships(&rels) {
            let kind = rel.kind.rsplit('/').next().unwrap_or_default();
            let attachment = match (kind, rel.external) {
                ("video" | "audio" | "media", false) => {
                    Attachment::Inside(join(folder, &rel.target))
                }
                ("video" | "audio" | "media" | "hyperlink", true)
                    if rel.target.starts_with("http://") || rel.target.starts_with("https://") =>
                {
                    Attachment::Link(rel.target)
                }
                _ => continue,
            };
            // A video is listed twice, as a video and as media.
            if !found.contains(&attachment) {
                found.push(attachment);
            }
        }
        slides.push(found);
    }
    Some(slides)
}

struct Relationship {
    id: String,
    /// Its type, a long address ending in what it is: `…/video`.
    kind: String,
    target: String,
    /// Whether the target is outside the file: a link, not a part of it.
    external: bool,
}

fn relationships(xml: &str) -> Vec<Relationship> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    doc.descendants()
        .filter(|node| node.has_tag_name("Relationship"))
        .filter_map(|node| {
            Some(Relationship {
                id: node.attribute("Id")?.to_owned(),
                kind: node.attribute("Type")?.to_owned(),
                target: node.attribute("Target")?.to_owned(),
                external: node.attribute("TargetMode") == Some("External"),
            })
        })
        .collect()
}

/// The relationship ids of the slides, in the order they are shown.
fn slide_ids(xml: &str) -> Vec<String> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    doc.descendants()
        .filter(|node| node.has_tag_name("sldId"))
        .filter_map(|node| {
            node.attributes()
                .find(|attribute| attribute.name() == "id" && attribute.namespace().is_some())
                .map(|attribute| attribute.value().to_owned())
        })
        .collect()
}

/// Whether a slide is marked not to be shown.
fn hidden(xml: &str) -> bool {
    roxmltree::Document::parse(xml)
        .is_ok_and(|doc| doc.root_element().attribute("show") == Some("0"))
}

/// `target` as a name in the zip, from the folder of the part that names
/// it: `ppt/slides` + `../media/a.mp4` is `ppt/media/a.mp4`.
fn join(folder: &str, target: &str) -> String {
    if let Some(absolute) = target.strip_prefix('/') {
        return absolute.to_owned();
    }
    let mut parts: Vec<&str> = folder.split('/').filter(|part| !part.is_empty()).collect();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// Where the file `entry` of the deck at `path` is kept once taken out of
/// it: in the deck's folder under `cache`, the one its PDF is in.
pub fn inside_path(path: &Path, entry: &str, cache: &Path) -> Option<PathBuf> {
    let name = Path::new(entry).file_name()?;
    Some(folder(path, cache).ok()?.join("media").join(name))
}

/// Takes `entry` out of the deck at `path` into [`inside_path`], unless
/// it is there already, and says where it is.
pub fn take_out(path: &Path, entry: &str, cache: &Path) -> Result<PathBuf, String> {
    let fail = |err: &dyn std::fmt::Display| format!("{}: {entry}: {err}", path.display());
    let out = inside_path(path, entry, cache).ok_or_else(|| fail(&"not a file"))?;
    if out.is_file() {
        return Ok(out);
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|err| fail(&err))?;
    }
    let file = std::fs::File::open(path).map_err(|err| fail(&err))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|err| fail(&err))?;
    let mut inside = zip.by_name(entry).map_err(|err| fail(&err))?;
    // Through a partial file, so one cut short is never taken for whole.
    let partial = out.with_extension("part");
    let mut copy = std::fs::File::create(&partial).map_err(|err| fail(&err))?;
    std::io::copy(&mut inside, &mut copy).map_err(|err| fail(&err))?;
    std::fs::rename(&partial, &out).map_err(|err| fail(&err))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decks_are_told_by_their_extension() {
        for deck in ["a.pptx", "a.PPTX", "b.ppt", "c.odp", "d.ppsx", "e.pps"] {
            assert!(is_deck(Path::new(deck)), "{deck}");
        }
        for other in ["a.pdf", "a.key", "pptx", "a.pptx.txt"] {
            assert!(!is_deck(Path::new(other)), "{other}");
        }
    }

    /// A `.pptx` with only what [`attachments`] reads: three slides shown
    /// out of file order, the second hidden.
    fn deck(dir: &Path) -> PathBuf {
        use std::io::Write;
        let path = dir.join("deck.pptx");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let packed = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        let rels = |body: &str| {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
            )
        };
        let slide = |show: &str| {
            format!(
                r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"{show}><p:cSld/></p:sld>"#
            )
        };
        let files = [
            (
                "ppt/presentation.xml".to_string(),
                format!(
                    r#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="{rel}"><p:sldIdLst><p:sldId id="258" r:id="rId4"/><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst></p:presentation>"#
                ),
            ),
            (
                "ppt/_rels/presentation.xml.rels".to_string(),
                rels(&format!(
                    r#"<Relationship Id="rId1" Type="{rel}/slideMaster" Target="slideMasters/slideMaster1.xml"/><Relationship Id="rId2" Type="{rel}/slide" Target="slides/slide1.xml"/><Relationship Id="rId3" Type="{rel}/slide" Target="slides/slide2.xml"/><Relationship Id="rId4" Type="{rel}/slide" Target="slides/slide3.xml"/>"#
                )),
            ),
            ("ppt/slides/slide1.xml".to_string(), slide(r#" show="0""#)),
            ("ppt/slides/slide2.xml".to_string(), slide("")),
            ("ppt/slides/slide3.xml".to_string(), slide("")),
            (
                "ppt/slides/_rels/slide1.xml.rels".to_string(),
                rels(&format!(
                    r#"<Relationship Id="rId1" Type="{rel}/video" Target="https://www.youtube.com/embed/hidden00000" TargetMode="External"/>"#
                )),
            ),
            (
                "ppt/slides/_rels/slide2.xml.rels".to_string(),
                rels(&format!(
                    r#"<Relationship Id="rId1" Type="{rel}/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="{rel}/image" Target="../media/image1.png"/><Relationship Id="rId3" Type="{rel}/video" Target="../media/media1.mp4"/><Relationship Id="rId4" Type="http://schemas.microsoft.com/office/2007/relationships/media" Target="../media/media1.mp4"/><Relationship Id="rId5" Type="{rel}/hyperlink" Target="https://youtu.be/dQw4w9WgXcQ" TargetMode="External"/><Relationship Id="rId6" Type="{rel}/hyperlink" Target="mailto:ana@example.org" TargetMode="External"/>"#
                )),
            ),
            (
                "ppt/slides/_rels/slide3.xml.rels".to_string(),
                rels(&format!(
                    r#"<Relationship Id="rId1" Type="{rel}/video" Target="https://www.youtube.com/embed/aqz-KE-bpKQ?feature=oembed" TargetMode="External"/>"#
                )),
            ),
            ("ppt/media/media1.mp4".to_string(), "a video".repeat(500)),
        ];
        for (name, body) in files {
            zip.start_file(name, packed).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn a_deck_lists_what_each_shown_slide_plays_or_links_to() {
        let dir = tempfile::tempdir().unwrap();
        let slides = attachments(&deck(dir.path()));
        assert_eq!(
            slides,
            [
                // The third file is the first slide shown.
                vec![Attachment::Link(
                    "https://www.youtube.com/embed/aqz-KE-bpKQ?feature=oembed".into()
                )],
                // The hidden slide is skipped: this one is the second page.
                vec![
                    Attachment::Inside("ppt/media/media1.mp4".into()),
                    Attachment::Link("https://youtu.be/dQw4w9WgXcQ".into()),
                ],
            ]
        );
    }

    #[test]
    fn what_the_stage_can_show_becomes_resources() {
        let dir = tempfile::tempdir().unwrap();
        let path = deck(dir.path());
        let cache = dir.path().join("cache");
        let resource = Resource::new(
            ResourceId::new(&SourceId::new("local").unwrap(), "deck"),
            "Culto",
            Kind::Presentation,
            Location::File(path.clone()),
        );
        let slides = attached(&resource, &cache);
        let titles: Vec<Vec<&str>> = slides
            .iter()
            .map(|slide| slide.iter().map(|r| r.title.as_str()).collect())
            .collect();
        assert_eq!(
            titles,
            [
                vec!["YouTube video aqz-KE-bpKQ"],
                vec!["Culto · slide 2", "YouTube video dQw4w9WgXcQ"],
            ]
        );
        let inside = &slides[1][0];
        assert_eq!(inside.kind, Kind::Video);
        assert_eq!(
            inside.location,
            Location::File(inside_path(&path, "ppt/media/media1.mp4", &cache).unwrap())
        );
        assert_eq!(inside.meta[ENTRY], "ppt/media/media1.mp4");
    }

    #[test]
    fn a_file_that_isnt_a_deck_has_no_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.pptx");
        std::fs::write(&path, "not a zip").unwrap();
        assert!(attachments(&path).is_empty());
        assert!(attachments(Path::new("/nope/lantern.pptx")).is_empty());
    }

    #[test]
    fn a_video_inside_a_deck_is_taken_out_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = deck(dir.path());
        let cache = dir.path().join("cache");
        let out = take_out(&path, "ppt/media/media1.mp4", &cache).unwrap();
        assert_eq!(out.file_name().unwrap(), "media1.mp4");
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "a video".repeat(500)
        );
        assert_eq!(
            inside_path(&path, "ppt/media/media1.mp4", &cache),
            Some(out.clone())
        );
        // The copy there is used as it is.
        std::fs::write(&out, "kept").unwrap();
        assert_eq!(
            take_out(&path, "ppt/media/media1.mp4", &cache).unwrap(),
            out
        );
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "kept");
        assert!(take_out(&path, "ppt/media/gone.mp4", &cache).is_err());
    }

    #[test]
    fn parts_are_named_from_the_part_that_points_to_them() {
        assert_eq!(join("ppt/slides", "../media/a.mp4"), "ppt/media/a.mp4");
        assert_eq!(join("ppt", "slides/slide1.xml"), "ppt/slides/slide1.xml");
        assert_eq!(join("ppt/slides", "/ppt/media/a.mp4"), "ppt/media/a.mp4");
    }

    #[test]
    fn no_libreoffice_says_so() {
        let missing = || std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(office(|_| Err(missing())).unwrap_err(), NO_OFFICE);
        let denied = || std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(
            office(|_| Err(denied()))
                .unwrap_err()
                .starts_with("soffice: "),
            "another failure is told as it is"
        );
    }

    #[test]
    fn a_deck_that_isnt_there_fails_before_libreoffice_is_asked() {
        let cache = tempfile::tempdir().unwrap();
        let err = to_pdf(Path::new("/nope/lantern.pptx"), cache.path()).unwrap_err();
        assert!(err.starts_with("/nope/lantern.pptx: "), "{err}");
    }
}
