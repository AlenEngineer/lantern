//! Thumbnails for results and plan steps: a small picture of what a
//! resource looks like. Made on two background threads and kept for the
//! session; text has none, since its first slide is drawn live instead.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::widget::image::Handle;
use lantern_engine::{Kind, Location, Resource, ResourceId};

use crate::stage;
use crate::youtube::YoutubeVideo;

/// The largest a thumbnail is drawn, in pixels; made at twice that for
/// sharp edges on a HiDPI screen.
const WIDTH: u32 = 192;
const HEIGHT: u32 = 108;
const WORKERS: usize = 2;

/// What a thumbnail is made from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    Image(PathBuf),
    Pdf(PathBuf),
    Video(PathBuf),
    /// Image links, tried in order (a YouTube video's still, a search
    /// result's picture and then its thumbnail).
    Links(Vec<String>),
}

impl Source {
    /// What `resource`'s thumbnail comes from, or `None` when it has none.
    fn of(resource: &Resource) -> Option<Source> {
        if let Some(video) = YoutubeVideo::of(resource) {
            let still = format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", video.id);
            return Some(Source::Links(vec![still]));
        }
        let thumbnail = resource.meta.get(Resource::THUMBNAIL).cloned();
        match (&resource.location, &resource.kind) {
            (Location::File(path), Kind::Image) => Some(Source::Image(path.clone())),
            (Location::File(path), Kind::Video) => Some(Source::Video(path.clone())),
            (Location::File(path), Kind::Document)
                if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("pdf")) =>
            {
                Some(Source::Pdf(path.clone()))
            }
            (Location::Url(url), Kind::Image) => Some(Source::Links(
                thumbnail.into_iter().chain([url.clone()]).collect(),
            )),
            _ => thumbnail.map(|link| Source::Links(vec![link])),
        }
    }

    fn make(&self) -> Result<Handle, String> {
        match self {
            Source::Image(path) => shrink(image::open(path).map_err(|e| e.to_string())?),
            Source::Pdf(path) => first_page(path),
            Source::Video(path) => video_still(path),
            Source::Links(links) => {
                let dir = stage::cache_dir().join("images");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let mut failures = Vec::new();
                for link in links {
                    let path = stage::image_path(link, &dir);
                    let fetched = if path.is_file() {
                        Ok(())
                    } else {
                        stage::download(link, &path)
                    };
                    match fetched
                        .and_then(|()| image::open(&path).map_err(|e| e.to_string()))
                        .and_then(shrink)
                    {
                        Ok(handle) => return Ok(handle),
                        Err(why) => failures.push(why),
                    }
                }
                Err(failures.join("; "))
            }
        }
    }
}

/// `picture` scaled to fit the thumbnail size.
fn shrink(picture: image::DynamicImage) -> Result<Handle, String> {
    let small = picture.thumbnail(WIDTH * 2, HEIGHT * 2).to_rgba8();
    Ok(Handle::from_rgba(
        small.width(),
        small.height(),
        small.into_raw(),
    ))
}

/// A PDF's first page, rendered small with `pdftoppm` into the cache,
/// named by the file's path.
fn first_page(path: &Path) -> Result<Handle, String> {
    let dir = stage::cache_dir().join("thumbs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut key = DefaultHasher::new();
    path.hash(&mut key);
    let out = dir.join(format!("{:016x}", key.finish()));
    let output = std::process::Command::new("pdftoppm")
        .args(["-png", "-f", "1", "-l", "1", "-singlefile", "-scale-to"])
        .arg((HEIGHT * 2).to_string())
        .arg(path)
        .arg(&out)
        .output()
        .map_err(|err| format!("pdftoppm: {err}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    shrink(image::open(out.with_extension("png")).map_err(|e| e.to_string())?)
}

/// A frame a little way into a video, so it isn't a black fade-in.
fn video_still(path: &Path) -> Result<Handle, String> {
    let uri = url::Url::from_file_path(path).map_err(|()| "not a file path".to_string())?;
    let mut video = stage::open_video(&uri)?;
    let at = video.duration().mul_f32(0.1).min(Duration::from_secs(10));
    let size = video.size();
    let downscale = (size.0 as u32 / (WIDTH * 2)).clamp(1, 255) as u8;
    let mut stills = video
        .thumbnails(
            [iced_video_player::Position::Time(at)],
            NonZeroU8::new(downscale).unwrap_or(NonZeroU8::MIN),
        )
        .map_err(|err| err.to_string())?;
    stills.pop().ok_or_else(|| "no frame".to_string())
}

#[derive(Debug, Clone)]
enum Slot {
    Making,
    Ready(Handle),
    None,
}

/// The thumbnails made so far, and the queue of ones to make.
pub struct Thumbs {
    slots: Arc<Mutex<HashMap<ResourceId, Slot>>>,
    queue: Sender<(ResourceId, Source)>,
}

impl Thumbs {
    pub fn start() -> Thumbs {
        let slots: Arc<Mutex<HashMap<ResourceId, Slot>>> = Arc::default();
        let (queue, jobs) = channel::<(ResourceId, Source)>();
        let jobs = Arc::new(Mutex::new(jobs));
        for _ in 0..WORKERS {
            let (slots, jobs) = (slots.clone(), jobs.clone());
            std::thread::spawn(move || work(&slots, &jobs));
        }
        Thumbs { slots, queue }
    }

    /// `resource`'s thumbnail once made. Asks for it the first time, so a
    /// view can call this for every row it draws.
    pub fn get(&self, resource: &Resource) -> Option<Handle> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        match slots.get(&resource.id) {
            Some(Slot::Ready(handle)) => return Some(handle.clone()),
            Some(Slot::Making | Slot::None) => return None,
            None => {}
        }
        match Source::of(resource) {
            Some(source) => {
                slots.insert(resource.id.clone(), Slot::Making);
                let _ = self.queue.send((resource.id.clone(), source));
            }
            None => {
                slots.insert(resource.id.clone(), Slot::None);
            }
        }
        None
    }

    /// Whether any thumbnail is still being made, so the view redraws.
    pub fn busy(&self) -> bool {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.values().any(|slot| matches!(slot, Slot::Making))
    }
}

fn work(slots: &Mutex<HashMap<ResourceId, Slot>>, jobs: &Mutex<Receiver<(ResourceId, Source)>>) {
    loop {
        let job = jobs.lock().unwrap_or_else(|e| e.into_inner()).recv();
        let Ok((id, source)) = job else { return };
        let slot = match source.make() {
            Ok(handle) => Slot::Ready(handle),
            Err(why) => {
                crate::log(format!("thumbs: {id}: {why}"));
                Slot::None
            }
        };
        slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, slot);
    }
}

#[cfg(test)]
mod tests {
    use lantern_engine::SourceId;

    use super::*;

    fn resource(kind: Kind, location: Location) -> Resource {
        let source = SourceId::new("test").unwrap();
        Resource::new(ResourceId::new(&source, "x"), "X", kind, location)
    }

    #[test]
    fn each_kind_takes_its_thumbnail_from_the_right_place() {
        let file = |name: &str| Location::File(PathBuf::from(name));
        assert_eq!(
            Source::of(&resource(Kind::Image, file("/a.jpg"))),
            Some(Source::Image("/a.jpg".into()))
        );
        assert_eq!(
            Source::of(&resource(Kind::Document, file("/a.PDF"))),
            Some(Source::Pdf("/a.PDF".into()))
        );
        assert_eq!(Source::of(&resource(Kind::Document, file("/a.docx"))), None);
        assert_eq!(
            Source::of(&resource(Kind::Text, Location::Inline("hi".into()))),
            None,
            "text draws its first slide instead"
        );
        let link = SourceId::new("link").unwrap();
        let youtube = lantern_source_link::youtube_video(&link, "dQw4w9WgXcQ");
        assert_eq!(
            Source::of(&youtube),
            Some(Source::Links(vec![
                "https://i.ytimg.com/vi/dQw4w9WgXcQ/mqdefault.jpg".into()
            ]))
        );
        let picture = resource(Kind::Image, Location::Url("https://x/big.jpg".into()))
            .with_meta(Resource::THUMBNAIL, "https://x/small.jpg");
        assert_eq!(
            Source::of(&picture),
            Some(Source::Links(vec![
                "https://x/small.jpg".into(),
                "https://x/big.jpg".into()
            ]))
        );
    }

    #[test]
    fn pictures_and_pdf_pages_come_out_small() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/e2e/fixtures");
        for source in [
            Source::Image(fixtures.join("image.png")),
            Source::Pdf(fixtures.join("pages.pdf")),
        ] {
            let handle = source.make().unwrap_or_else(|e| panic!("{source:?}: {e}"));
            let iced::advanced::image::Handle::Rgba { width, height, .. } = handle else {
                panic!("{source:?}: not decoded");
            };
            assert!(
                width <= WIDTH * 2 && height <= HEIGHT * 2,
                "{width}×{height}"
            );
        }
    }
}
