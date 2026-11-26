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

use crate::pixels::Pixels;
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
    /// A deck, which carries a picture of its first slide.
    Deck(PathBuf),
    Video(PathBuf),
    /// Image links, tried in order: stand-ins first (a YouTube video's
    /// still, a search result's thumbnail), then the picture itself where
    /// the resource is one.
    Links {
        stand_ins: Vec<String>,
        itself: Option<String>,
    },
}

impl Source {
    /// What `resource`'s thumbnail comes from, or `None` when it has none.
    fn of(resource: &Resource) -> Option<Source> {
        if let Some(video) = YoutubeVideo::of(resource) {
            let still = format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", video.id);
            return Some(Source::Links {
                stand_ins: vec![still],
                itself: None,
            });
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
            (Location::File(path), Kind::Presentation) if crate::deck::is_deck(path) => {
                Some(Source::Deck(path.clone()))
            }
            (Location::Url(url), Kind::Image) => Some(Source::Links {
                stand_ins: thumbnail.into_iter().collect(),
                itself: Some(url.clone()),
            }),
            _ => thumbnail.map(|link| Source::Links {
                stand_ins: vec![link],
                itself: None,
            }),
        }
    }

    /// The thumbnail, and the size of what it is a picture of where that
    /// was opened: a picture's or a video's own, never a stand-in's.
    fn make(&self) -> Result<(Made, Option<Pixels>), String> {
        match self {
            Source::Image(path) => {
                let picture = open(path)?;
                let pixels = pixels_of(&picture);
                Ok((shrink(picture)?, Some(pixels)))
            }
            Source::Pdf(path) => Ok((first_page(path)?, None)),
            Source::Deck(path) => {
                let picture = crate::deck::thumbnail(path)?;
                let picture = image::load_from_memory(&picture).map_err(|e| e.to_string())?;
                Ok((shrink(picture)?, None))
            }
            Source::Video(path) => {
                let (still, pixels) = video_still(path)?;
                Ok((shrink(still)?, Some(pixels)))
            }
            Source::Links { stand_ins, itself } => {
                let dir = stage::cache_dir().join("images");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let links = stand_ins.iter().map(|link| (link, false));
                let mut failures = Vec::new();
                for (link, own) in links.chain(itself.iter().map(|link| (link, true))) {
                    let path = stage::image_path(link, &dir);
                    let fetched = if path.is_file() {
                        Ok(())
                    } else {
                        stage::download(link, &path)
                    };
                    match fetched.and_then(|()| open(&path)) {
                        Ok(picture) => {
                            let pixels = own.then(|| pixels_of(&picture));
                            return Ok((shrink(picture)?, pixels));
                        }
                        Err(why) => failures.push(why),
                    }
                }
                Err(failures.join("; "))
            }
        }
    }
}

fn pixels_of(picture: &image::DynamicImage) -> Pixels {
    Pixels {
        width: picture.width(),
        height: picture.height(),
    }
}

/// The picture at `path`, its format read from its first bytes: the
/// download cache names files by a hash, with no extension to go by.
fn open(path: &Path) -> Result<image::DynamicImage, String> {
    image::ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())
}

/// A thumbnail as made, both ways scaled to fit the thumbnail size.
#[derive(Debug, Clone)]
struct Made {
    /// Cut to 16:9. Most frames a thumbnail is drawn in are 16:9, and a
    /// picture of that shape fills one exactly, so the picture's rounded
    /// corners are the frame's.
    wide: Handle,
    /// Whole, for a tile drawn at the picture's own shape. For a picture
    /// that is 16:9 already, the same as `wide`.
    whole: Handle,
    /// `whole`'s width over its height.
    shape: f32,
}

fn shrink(picture: image::DynamicImage) -> Result<Made, String> {
    let small = |picture: &image::DynamicImage| {
        let small = picture.thumbnail(WIDTH * 2, HEIGHT * 2).to_rgba8();
        let shape = small.width() as f32 / small.height() as f32;
        let handle = Handle::from_rgba(small.width(), small.height(), small.into_raw());
        (handle, shape)
    };
    let (whole, shape) = small(&picture);
    let wide = match widescreen(&picture) {
        Some(cut) => small(&cut).0,
        None => whole.clone(),
    };
    Ok(Made { wide, whole, shape })
}

/// The middle of `picture` at 16:9, what a 16:9 frame filled with it
/// shows; none for a picture of that shape already.
fn widescreen(picture: &image::DynamicImage) -> Option<image::DynamicImage> {
    let (width, height) = (picture.width(), picture.height());
    let wide = (u64::from(height) * 16 / 9).max(1) as u32;
    let tall = (u64::from(width) * 9 / 16).max(1) as u32;
    if width > wide {
        Some(picture.crop_imm((width - wide) / 2, 0, wide, height))
    } else if height > tall {
        Some(picture.crop_imm(0, (height - tall) / 2, width, tall))
    } else {
        None
    }
}

/// A PDF's first page, rendered as wide as a thumbnail with `pdftoppm`
/// into the cache, named by the file's path.
fn first_page(path: &Path) -> Result<Made, String> {
    let dir = stage::cache_dir().join("thumbs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut key = DefaultHasher::new();
    path.hash(&mut key);
    let out = dir.join(format!("{:016x}", key.finish()));
    let output = crate::programs::command("pdftoppm")
        .args(["-png", "-f", "1", "-l", "1", "-singlefile"])
        .args(["-scale-to-y", "-1", "-scale-to-x"])
        .arg((WIDTH * 2).to_string())
        .arg(path)
        .arg(&out)
        .output()
        .map_err(|err| format!("pdftoppm: {err}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    shrink(open(&out.with_extension("png"))?)
}

/// A frame a little way into a video, so it isn't a black fade-in, and
/// the video's size.
fn video_still(path: &Path) -> Result<(image::DynamicImage, Pixels), String> {
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
    let Some(Handle::Rgba {
        width,
        height,
        pixels: frame,
        ..
    }) = stills.pop()
    else {
        return Err("no frame".to_string());
    };
    let still = image::RgbaImage::from_raw(width, height, frame.to_vec())
        .ok_or_else(|| "a frame of the wrong size".to_string())?;
    let pixels = Pixels {
        width: size.0 as u32,
        height: size.1 as u32,
    };
    Ok((image::DynamicImage::ImageRgba8(still), pixels))
}

#[derive(Debug, Clone)]
enum Slot {
    Making,
    /// Made, with the size of what it is a picture of, where known.
    Ready(Made, Option<Pixels>),
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

    /// `resource`'s thumbnail cut to 16:9, once made. Asks for it the
    /// first time, so a view can call this for every row it draws.
    pub fn get(&self, resource: &Resource) -> Option<Handle> {
        self.made(resource).map(|made| made.wide)
    }

    /// `resource`'s thumbnail whole, and its shape, its width over its
    /// height, once made. Asks for it as [`Thumbs::get`] does.
    pub fn whole(&self, resource: &Resource) -> Option<(Handle, f32)> {
        self.made(resource).map(|made| (made.whole, made.shape))
    }

    fn made(&self, resource: &Resource) -> Option<Made> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        match slots.get(&resource.id) {
            Some(Slot::Ready(made, _)) => return Some(made.clone()),
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

    /// The size of what `id`'s thumbnail is a picture of, once made: a
    /// picture's or a video's own.
    pub fn pixels(&self, id: &ResourceId) -> Option<Pixels> {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        match slots.get(id)? {
            Slot::Ready(_, pixels) => *pixels,
            Slot::Making | Slot::None => None,
        }
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
            Ok((made, pixels)) => Slot::Ready(made, pixels),
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
    fn a_cached_picture_opens_without_an_extension() {
        let dir = std::env::temp_dir().join(format!("lantern-thumbs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("88c814cb25463dc3");
        image::RgbaImage::new(4, 3)
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        let picture = open(&path);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(picture.map(|p| (p.width(), p.height())), Ok((4, 3)));
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
            Some(Source::Links {
                stand_ins: vec!["https://i.ytimg.com/vi/dQw4w9WgXcQ/mqdefault.jpg".into()],
                itself: None,
            }),
            "a still stands in for the video"
        );
        let picture = resource(Kind::Image, Location::Url("https://x/big.jpg".into()))
            .with_meta(Resource::THUMBNAIL, "https://x/small.jpg");
        assert_eq!(
            Source::of(&picture),
            Some(Source::Links {
                stand_ins: vec!["https://x/small.jpg".into()],
                itself: Some("https://x/big.jpg".into()),
            })
        );
    }

    #[test]
    fn thumbnails_come_out_small_cut_to_16_9_and_whole_at_their_own_shape() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/e2e/fixtures");
        let pixels = |width, height| Some(Pixels { width, height });
        for (source, size, shape) in [
            (
                Source::Image(fixtures.join("image.png")),
                pixels(800, 450),
                16.0 / 9.0,
            ),
            (Source::Pdf(fixtures.join("pages.pdf")), None, 0.75),
            (
                Source::Image(fixtures.join("portrait.jpg")),
                pixels(1080, 1350),
                0.8,
            ),
        ] {
            let (made, pixels) = source.make().unwrap_or_else(|e| panic!("{source:?}: {e}"));
            let made_as = |handle: &Handle| match handle {
                Handle::Rgba { width, height, .. } => (*width, *height),
                _ => panic!("{source:?}: not decoded"),
            };
            assert_eq!(
                made_as(&made.wide),
                (WIDTH * 2, HEIGHT * 2),
                "{source:?}: as big as a thumbnail is made, at 16:9"
            );
            let (width, height) = made_as(&made.whole);
            assert!(
                width <= WIDTH * 2 && height <= HEIGHT * 2,
                "{source:?}: {width}×{height}"
            );
            assert!(
                (width as f32 / height as f32 - shape).abs() < 0.01
                    && (made.shape - shape).abs() < 0.01,
                "{source:?}: whole, at its own shape, is {width}×{height}"
            );
            assert_eq!(pixels, size, "{source:?}");
        }
    }
}
