//! The stage window's renderer. It loads what the program scene needs
//! (decoded image, video player, rendered PDF pages), keeps the player in
//! step with the engine's playback, and reports what it learned back with
//! [`Engine::report`].
//!
//! Loading happens on worker threads, keyed by [`ResourceId`]. The same path
//! serves the program and the preload of what comes next (the preview and
//! the plan's next step), so a take of a preloaded video only swaps a player
//! that is already built.
//!
//! A YouTube video loads like a local one, from the download cache. Until
//! its download is done, it streams instead: the loader asks `yt-dlp` for
//! the stream (a few seconds) and the download goes on for next time. If it
//! can't stream, it waits for the download. An image at a link is downloaded into the cache
//! first, falling back to its [thumbnail](Resource::THUMBNAIL) when the
//! site refuses.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use iced::widget::image::Handle;
use iced_video_player::Video;
use lantern_engine::{
    Engine, Kind, Location, MediaReport, Playback, Resource, ResourceId, SceneId, StageState, Timed,
};

use crate::youtube::{Downloads, YoutubeVideo};

/// What the stage has loaded for one resource.
pub enum Content {
    /// A decoded still image.
    Image(Handle),
    /// A player, paused and muted until it goes on program.
    Video {
        player: Video,
        /// It plays a YouTube stream, not a file. GStreamer can hang
        /// seeking a stream, so a stream is never seeked: no scrubbing and
        /// no replay until the download replaces it.
        stream: bool,
    },
    /// Rendered PDF pages, in order. Never empty.
    Pages(Vec<Handle>),
}

/// How the stage presents a resource. Decided from kind and location alone.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Loader {
    Image(PathBuf),
    /// An image at a link, then the links to try if it fails.
    RemoteImage(Vec<String>),
    Video(PathBuf),
    Pdf(PathBuf),
    Youtube(YoutubeVideo),
}

/// What the stage needs to show a resource, for checking a plan before
/// the service. Decided by the same rules the loader uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Needs {
    /// Inline text: nothing outside the plan.
    Nothing,
    /// A file on disk.
    File(PathBuf),
    /// A YouTube video's download (or the network, to stream it).
    Youtube(YoutubeVideo),
    /// One of these image links, downloaded or downloadable.
    Images(Vec<String>),
}

/// What showing `resource` takes, or why the stage can't show it.
pub fn needs(resource: &Resource) -> Result<Needs, String> {
    Ok(match Loader::for_resource(resource)? {
        None => Needs::Nothing,
        Some(Loader::Image(path) | Loader::Video(path) | Loader::Pdf(path)) => Needs::File(path),
        Some(Loader::Youtube(video)) => Needs::Youtube(video),
        Some(Loader::RemoteImage(urls)) => Needs::Images(urls),
    })
}

/// Whether an image link is already in the cache, so showing it needs no
/// network.
pub fn image_cached(url: &str) -> bool {
    image_path(url, &cache_dir().join("images")).is_file()
}

/// Where an image link is cached: a file in `dir` named by the link.
pub(crate) fn image_path(url: &str, dir: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    dir.join(format!("{:016x}", hasher.finish()))
}

impl Loader {
    /// `Ok(None)` for inline text, which the stage draws from the scene.
    /// `Err` says why the stage can't show the resource.
    fn for_resource(resource: &Resource) -> Result<Option<Loader>, String> {
        if let Some(video) = YoutubeVideo::of(resource) {
            return Ok(Some(Loader::Youtube(video)));
        }
        let path = match &resource.location {
            Location::Inline(_) => return Ok(None),
            Location::File(path) => path.clone(),
            Location::Url(url) if resource.kind == Kind::Image => {
                let thumbnail = resource.meta.get(Resource::THUMBNAIL).cloned();
                let urls = std::iter::once(url.clone()).chain(thumbnail).collect();
                return Ok(Some(Loader::RemoteImage(urls)));
            }
            Location::Url(url) => {
                return Err(format!("the stage can't show this link yet ({url})"));
            }
        };
        let is_pdf = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"));
        match &resource.kind {
            Kind::Image => Ok(Some(Loader::Image(path))),
            Kind::Video => Ok(Some(Loader::Video(path))),
            Kind::Document if is_pdf => Ok(Some(Loader::Pdf(path))),
            kind => Err(format!("the stage can't show {} files", kind.name())),
        }
    }

    /// Blocks: decodes, builds a player, runs `pdftoppm`, or waits for or
    /// makes a download.
    fn load(&self, downloads: &Arc<Downloads>) -> Result<Content, String> {
        match self {
            Loader::Image(path) => decode_image(existing(path)?).map(Content::Image),
            Loader::RemoteImage(urls) => {
                fetch_image(urls, &cache_dir().join("images")).map(Content::Image)
            }
            Loader::Video(path) => open_file(path),
            Loader::Youtube(video) => {
                if let Some(path) = downloads.file(video) {
                    return open_file(&path);
                }
                crate::log(format!(
                    "stage: streaming {} until it's downloaded",
                    video.id
                ));
                match crate::youtube::stream_url(video).and_then(|uri| open_video(&uri)) {
                    Ok(player) => Ok(Content::Video {
                        player,
                        stream: true,
                    }),
                    Err(err) => {
                        crate::log(format!(
                            "stage: can't stream {} ({err}), waiting for the download",
                            video.id
                        ));
                        open_file(&downloads.ready(video)?)
                    }
                }
            }
            Loader::Pdf(path) => {
                render_pdf(existing(path)?, &cache_dir().join("pdf")).map(Content::Pages)
            }
        }
    }
}

/// A player for a video file, at its start.
fn open_file(path: &Path) -> Result<Content, String> {
    let uri = url::Url::from_file_path(existing(path)?)
        .map_err(|()| format!("{}: not an absolute path", path.display()))?;
    let mut player = open_video(&uri)?;
    player
        .seek(Duration::ZERO, true)
        .map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(Content::Video {
        player,
        stream: false,
    })
}

fn existing(path: &Path) -> Result<&Path, String> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("file not found: {}", path.display()))
    }
}

/// A finished load, sent from the worker thread. The content is taken out
/// once; `Message` must be `Clone`, and a player can't be.
pub struct Loaded {
    resource: ResourceId,
    result: Mutex<Option<Result<Content, String>>>,
}

impl fmt::Debug for Loaded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Loaded({})", self.resource)
    }
}

/// Starts loading on a worker thread.
fn spawn(
    resource: ResourceId,
    loader: Loader,
    downloads: Arc<Downloads>,
) -> iced::Task<Arc<Loaded>> {
    iced::Task::future(async move {
        let (done, result) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = done.send(loader.load(&downloads));
        });
        let result = result
            .await
            .unwrap_or_else(|_| Err("the loader crashed".into()));
        Arc::new(Loaded {
            resource,
            result: Mutex::new(Some(result)),
        })
    })
}

/// The program slot as the renderer sees it.
enum Program {
    /// Nothing to load: an empty stage, inline text, or something the stage can't show.
    Bare,
    Loading {
        scene: SceneId,
        resource: ResourceId,
    },
    Showing {
        scene: SceneId,
        content: Content,
    },
}

/// The renderer's state. The engine's mirror ([`StageState`]) stays the
/// authority on what is shown; this only holds what it takes to draw it.
pub struct Stage {
    program: Program,
    /// Loaded ahead of time for the preview or the next plan step.
    ready: HashMap<ResourceId, Content>,
    /// Loads in flight, for the program or a preload.
    loading: HashSet<ResourceId>,
    /// What the last [`preload`](Stage::preload) asked for.
    upcoming: HashSet<ResourceId>,
    /// The real length of every video loaded so far, for timing the plan.
    lengths: HashMap<ResourceId, Duration>,
    downloads: Arc<Downloads>,
}

impl Stage {
    pub fn new(downloads: Arc<Downloads>) -> Self {
        Stage {
            downloads,
            program: Program::Bare,
            ready: HashMap::new(),
            loading: HashSet::new(),
            upcoming: HashSet::new(),
            lengths: HashMap::new(),
        }
    }

    /// The content for the program scene, once it has loaded.
    pub fn content(&self, scene: SceneId) -> Option<&Content> {
        match &self.program {
            Program::Showing {
                scene: shown,
                content,
            } if *shown == scene => Some(content),
            _ => None,
        }
    }

    /// A video's real length, once it has loaded.
    pub fn length(&self, id: &ResourceId) -> Option<Duration> {
        self.lengths.get(id).copied()
    }

    /// What a preload has finished loading for `id`.
    pub fn preloaded(&self, id: &ResourceId) -> Option<&Content> {
        self.ready.get(id)
    }

    /// The program's player, while a video is on program.
    pub fn video(&mut self) -> Option<(SceneId, &mut Video)> {
        match &mut self.program {
            Program::Showing {
                scene,
                content: Content::Video { player, .. },
            } => Some((*scene, player)),
            _ => None,
        }
    }

    /// Whether the program plays a stream, which must not be seeked.
    fn streaming(&self) -> bool {
        matches!(
            self.program,
            Program::Showing {
                content: Content::Video { stream: true, .. },
                ..
            }
        )
    }

    /// A new scene is on program (or nothing). Uses a preloaded load when
    /// there is one, otherwise starts loading. Then drops preloads that are
    /// no longer upcoming. Only here: `take` empties the preview just before
    /// the program changes, so pruning on preview changes would drop the
    /// player about to be taken.
    pub fn program_changed(
        &mut self,
        stage: &StageState,
        engine: &Engine,
    ) -> iced::Task<Arc<Loaded>> {
        let task = self.show(stage, engine);
        self.ready.retain(|id, _| self.upcoming.contains(id));
        task
    }

    fn show(&mut self, stage: &StageState, engine: &Engine) -> iced::Task<Arc<Loaded>> {
        self.program = Program::Bare;
        let Some(scene) = &stage.program else {
            return iced::Task::none();
        };
        let loader = match Loader::for_resource(&scene.resource) {
            Ok(Some(loader)) => loader,
            Ok(None) => return iced::Task::none(),
            Err(reason) => {
                engine.report(scene.id, MediaReport::Failed { reason });
                return iced::Task::none();
            }
        };
        if let Loader::Youtube(video) = &loader {
            self.downloads.retry(video);
        }
        let resource = scene.resource.id.clone();
        if let Some(content) = self.ready.remove(&resource) {
            crate::log(format!("stage: {} was preloaded", scene.id));
            self.install(scene.id, Ok(content), stage, engine);
            return iced::Task::none();
        }
        self.program = Program::Loading {
            scene: scene.id,
            resource: resource.clone(),
        };
        if self.loading.insert(resource.clone()) {
            spawn(resource, loader, self.downloads.clone())
        } else {
            iced::Task::none()
        }
    }

    /// A load finished: it goes on program if the program is waiting for
    /// it, into `ready` if it is still upcoming, and is dropped otherwise.
    pub fn loaded(&mut self, loaded: &Loaded, stage: &StageState, engine: &Engine) {
        self.loading.remove(&loaded.resource);
        let Some(result) = loaded.result.lock().ok().and_then(|mut slot| slot.take()) else {
            return;
        };
        if let Ok(Content::Video { player, .. }) = &result
            && !player.duration().is_zero()
        {
            self.lengths
                .insert(loaded.resource.clone(), player.duration());
        }
        match &self.program {
            Program::Loading { scene, resource } if *resource == loaded.resource => {
                let scene = *scene;
                self.install(scene, result, stage, engine);
            }
            _ => {
                if let Ok(content) = result
                    && self.upcoming.contains(&loaded.resource)
                {
                    self.ready.insert(loaded.resource.clone(), content);
                }
            }
        }
    }

    fn install(
        &mut self,
        scene: SceneId,
        result: Result<Content, String>,
        stage: &StageState,
        engine: &Engine,
    ) {
        let mut content = match result {
            Ok(content) => content,
            Err(reason) => {
                crate::log(format!("stage: {scene} failed: {reason}"));
                engine.report(scene, MediaReport::Failed { reason });
                self.program = Program::Bare;
                return;
            }
        };
        match &mut content {
            Content::Image(_) => {}
            Content::Pages(pages) => {
                let count = NonZeroUsize::new(pages.len()).unwrap_or(NonZeroUsize::MIN);
                engine.report(scene, MediaReport::Frames(count));
            }
            Content::Video { player, .. } => {
                let duration = Some(player.duration()).filter(|d| !d.is_zero());
                engine.report(scene, MediaReport::Timeline { duration });
                player.set_muted(false);
            }
        }
        crate::log(format!("stage: {scene} ready"));
        self.program = Program::Showing { scene, content };
        if let Some(timed) = stage.program.as_ref().and_then(|scene| scene.timed()) {
            if !timed.position.is_zero() {
                self.seek(timed.position, engine);
            }
            self.sync_playback(timed, engine);
        }
    }

    /// Plays or pauses the player to match the engine.
    pub fn sync_playback(&mut self, timed: &Timed, engine: &Engine) {
        let streaming = self.streaming();
        let Some((scene, video)) = self.video() else {
            return;
        };
        // Playing an ended player restarts it with a seek.
        if streaming && video.eos() {
            return;
        }
        let paused = timed.playback != Playback::Playing;
        if video.paused() != paused {
            video.set_paused(paused);
        }
        if paused {
            engine.report(scene, MediaReport::Position(video.position()));
        }
    }

    /// Jumps the player and reports where it landed.
    pub fn seek(&mut self, position: Duration, engine: &Engine) {
        if self.streaming() {
            crate::log("stage: a stream can't seek until its download is ready");
            return;
        }
        let Some((scene, video)) = self.video() else {
            return;
        };
        if video.eos() {
            // Clears the player's end flag, which would otherwise make the
            // next play restart from zero instead of from `position`.
            let _ = video.restart_stream();
            video.set_paused(true);
        }
        if let Err(err) = video.seek(position, true) {
            crate::log(format!("stage: seek failed: {err}"));
        }
        engine.report(scene, MediaReport::Position(position));
    }

    /// Starts loading what may go on program next.
    pub fn preload(
        &mut self,
        upcoming: impl IntoIterator<Item = Resource>,
    ) -> iced::Task<Arc<Loaded>> {
        let loaders: Vec<_> = upcoming
            .into_iter()
            .filter_map(|resource| match Loader::for_resource(&resource) {
                Ok(Some(loader)) => Some((resource.id, loader)),
                _ => None,
            })
            .collect();
        self.upcoming = loaders.iter().map(|(id, _)| id.clone()).collect();
        let tasks: Vec<_> = loaders
            .into_iter()
            .filter(|(id, _)| !self.ready.contains_key(id))
            .filter(|(id, _)| self.loading.insert(id.clone()))
            .map(|(id, loader)| spawn(id, loader, self.downloads.clone()))
            .collect();
        iced::Task::batch(tasks)
    }
}

fn decode_image(path: &Path) -> Result<Handle, String> {
    let image = image::ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|err| format!("{}: {err}", path.display()))?
        .decode()
        .map_err(|err| format!("{}: {err}", path.display()))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Ok(Handle::from_rgba(width, height, image.into_raw()))
}

/// The biggest image download accepted, so a wrong link can't fill the cache.
const MAX_IMAGE_BYTES: u64 = 50 * 1024 * 1024;

/// Downloads and decodes the first of `urls` that works. Each download is
/// kept in `dir`, named by its link, so showing it again is offline.
fn fetch_image(urls: &[String], dir: &Path) -> Result<Handle, String> {
    let mut failures = Vec::new();
    for url in urls {
        let path = image_path(url, dir);
        let fetched = if path.is_file() {
            Ok(())
        } else {
            download(url, &path)
        };
        match fetched.and_then(|()| decode_image(&path)) {
            Ok(handle) => return Ok(handle),
            Err(err) => {
                let _ = std::fs::remove_file(&path);
                failures.push(err);
            }
        }
    }
    Err(failures.join("; "))
}

/// Saves `url` at `path`, through a temporary file so a cut-off download
/// never looks finished.
pub(crate) fn download(url: &str, path: &Path) -> Result<(), String> {
    let fail = |err: &dyn fmt::Display| format!("{url}: {err}");
    let bytes = ureq::get(url)
        .call()
        .and_then(|mut response| {
            response
                .body_mut()
                .with_config()
                .limit(MAX_IMAGE_BYTES)
                .read_to_vec()
        })
        .map_err(|err| fail(&err))?;
    let partial = path.with_extension("part");
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))
        .and_then(|()| std::fs::write(&partial, bytes))
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(|err| fail(&err))
}

/// Builds a player, paused and muted, so a preload is silent. [`Video::new`]
/// would start playing with sound while it prerolls. It may have played a
/// moment: [`open_file`] seeks a file back to its start.
///
/// A stream changes resolution as the network allows, but the player keeps
/// the size of its first frame, so a stream is scaled to one size (with
/// black bars to keep its shape).
pub(crate) fn open_video(uri: &url::Url) -> Result<Video, String> {
    let file = uri.to_file_path().ok();
    // A stream's link is long and signed: name it, don't print it.
    let name = file.as_ref().map_or_else(
        || "the stream".to_string(),
        |path| path.display().to_string(),
    );
    let fail = |err: &dyn fmt::Display| format!("{name}: {err}");
    let size = if file.is_some() {
        ""
    } else {
        ",width=1920,height=1080"
    };
    gst::init().map_err(|err| fail(&err))?;
    let pipeline = gst::parse::launch(&format!(
        "playbin uri=\"{uri}\" mute=true video-sink=\"videoscale add-borders=true ! \
         videoconvert ! appsink name=sink drop=true \
         caps=video/x-raw,format=NV12,pixel-aspect-ratio=1/1{size}\""
    ))
    .map_err(|err| fail(&err))?
    .downcast::<gst::Pipeline>()
    .map_err(|_| fail(&"not a pipeline"))?;
    let sink = pipeline
        .property::<gst::Element>("video-sink")
        .downcast::<gst::Bin>()
        .ok()
        .and_then(|bin| bin.by_name("sink"))
        .and_then(|sink| sink.downcast::<gstreamer_app::AppSink>().ok())
        .ok_or_else(|| fail(&"no video sink"))?;
    let mut video = Video::from_gst_pipeline(pipeline, sink, None).map_err(|err| fail(&err))?;
    video.set_paused(true);
    Ok(video)
}

/// Renders every page with `pdftoppm` into a folder under `cache`, keyed by
/// the file's path, size and modification time, and reuses it next time.
fn render_pdf(path: &Path, cache: &Path) -> Result<Vec<Handle>, String> {
    let fail = |err: &dyn fmt::Display| format!("{}: {err}", path.display());
    let meta = std::fs::metadata(path).map_err(|err| fail(&err))?;
    let mut key = DefaultHasher::new();
    (path, meta.len(), meta.modified().ok()).hash(&mut key);
    let dir = cache.join(format!("{:016x}", key.finish()));
    let done = dir.join("done");
    if !done.exists() {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|err| fail(&err))?;
        let output = std::process::Command::new("pdftoppm")
            .args(["-png", "-scale-to", "1920"])
            .arg(path)
            .arg(dir.join("page"))
            .output()
            .map_err(|err| fail(&format!("pdftoppm: {err}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(fail(&format!("pdftoppm: {}", stderr.trim())));
        }
        std::fs::write(&done, "").map_err(|err| fail(&err))?;
    }
    let mut pages: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|err| fail(&err))?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|page| page.extension().is_some_and(|ext| ext == "png"))
        .collect();
    // pdftoppm pads page numbers to the same width, so names sort as numbers.
    pages.sort();
    if pages.is_empty() {
        return Err(fail(&"no pages"));
    }
    Ok(pages.into_iter().map(Handle::from_path).collect())
}

pub fn cache_dir() -> PathBuf {
    crate::config::xdg_dir(crate::config::env, "XDG_CACHE_HOME", ".cache")
}

#[cfg(test)]
mod tests {
    use lantern_engine::SourceId;

    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/e2e/fixtures")
            .join(name)
    }

    fn resource(kind: Kind, location: Location) -> Resource {
        let source = SourceId::new("test").unwrap();
        Resource::new(ResourceId::new(&source, "x"), "X", kind, location)
    }

    #[test]
    fn the_kind_and_location_pick_how_to_present() {
        let file = |kind, name: &str| resource(kind, Location::File(fixture(name)));
        assert_eq!(
            Loader::for_resource(&file(Kind::Video, "video.mp4")),
            Ok(Some(Loader::Video(fixture("video.mp4"))))
        );
        assert_eq!(
            Loader::for_resource(&file(Kind::Document, "pages.pdf")),
            Ok(Some(Loader::Pdf(fixture("pages.pdf"))))
        );
        assert_eq!(
            Loader::for_resource(&resource(Kind::Text, Location::Inline("hi".into()))),
            Ok(None)
        );
        assert_eq!(
            Loader::for_resource(&file(Kind::Web, "page.html")),
            Err("the stage can't show web files".into())
        );
        assert!(Loader::for_resource(&file(Kind::Document, "notes.docx")).is_err());
    }

    #[test]
    fn an_image_link_loads_remotely_with_its_thumbnail_as_fallback() {
        let url = |kind| resource(kind, Location::Url("https://example.org/a.jpg".into()));
        let image = url(Kind::Image).with_meta(Resource::THUMBNAIL, "https://example.org/t.jpg");
        assert_eq!(
            Loader::for_resource(&image),
            Ok(Some(Loader::RemoteImage(vec![
                "https://example.org/a.jpg".into(),
                "https://example.org/t.jpg".into(),
            ])))
        );
        assert!(Loader::for_resource(&url(Kind::Web)).is_err());
    }

    /// Serves `image.png` at `/ok.png` and 404 for anything else, for
    /// `requests` connections. Returns its base URL.
    fn image_server(requests: usize) -> String {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let png = std::fs::read(fixture("image.png")).unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(requests) {
                let mut stream = stream.unwrap();
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).unwrap();
                let (status, body) = if line.starts_with("GET /ok.png ") {
                    ("200 OK", png.as_slice())
                } else {
                    ("404 Not Found", &b""[..])
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });
        base
    }

    #[test]
    fn a_remote_image_falls_back_to_the_next_link_and_is_cached() {
        let base = image_server(2);
        let cache = tempfile::tempdir().unwrap();
        let urls = [format!("{base}/gone.jpg"), format!("{base}/ok.png")];
        assert!(fetch_image(&urls, cache.path()).is_ok());
        let cached = std::fs::read_dir(cache.path()).unwrap().count();
        assert_eq!(cached, 1, "only the image that worked is kept");
        // The server answers only two requests; this one comes from the cache.
        assert!(fetch_image(&urls[1..], cache.path()).is_ok());

        let err = fetch_image(&[format!("{base}/gone.jpg")], cache.path()).unwrap_err();
        assert!(err.contains("gone.jpg"), "{err}");
    }

    #[test]
    fn a_missing_file_fails_with_its_path() {
        let downloads = Downloads::new("/nonexistent".into());
        let err = Loader::Image("/nonexistent/x.png".into())
            .load(&downloads)
            .err()
            .unwrap();
        assert_eq!(err, "file not found: /nonexistent/x.png");
    }

    #[test]
    fn images_decode_and_garbage_fails() {
        assert!(decode_image(&fixture("image.png")).is_ok());
        assert!(decode_image(&fixture("page.html")).is_err());
    }

    #[test]
    fn pdfs_render_one_image_per_page_and_reuse_the_cache() {
        let cache = tempfile::tempdir().unwrap();
        let pages = render_pdf(&fixture("pages.pdf"), cache.path()).unwrap();
        assert_eq!(pages.len(), 3);
        let again = render_pdf(&fixture("pages.pdf"), cache.path()).unwrap();
        assert_eq!(again, pages);
    }
}
