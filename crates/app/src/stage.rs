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
//! A YouTube video loads like a local one, from the download cache. Its
//! loader waits for the download, so a take before it is ready shows black
//! until the file arrives.

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
    Video(Video),
    /// Rendered PDF pages, in order. Never empty.
    Pages(Vec<Handle>),
}

/// How the stage presents a resource. Decided from kind and location alone.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Loader {
    Image(PathBuf),
    Video(PathBuf),
    Pdf(PathBuf),
    Youtube(YoutubeVideo),
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
            Location::Url(url) => return Err(format!("the stage can't show links yet ({url})")),
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

    /// Blocks: decodes, builds a player, runs `pdftoppm` or waits for a download.
    fn load(&self, downloads: &Arc<Downloads>) -> Result<Content, String> {
        let path = match self {
            Loader::Image(path) | Loader::Video(path) | Loader::Pdf(path) => path.clone(),
            Loader::Youtube(video) => downloads.ready(video)?,
        };
        if !path.is_file() {
            return Err(format!("file not found: {}", path.display()));
        }
        match self {
            Loader::Image(_) => decode_image(&path).map(Content::Image),
            Loader::Video(_) | Loader::Youtube(_) => open_video(&path).map(Content::Video),
            Loader::Pdf(_) => render_pdf(&path, &cache_dir().join("pdf")).map(Content::Pages),
        }
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

    /// What a preload has finished loading for `id`.
    pub fn preloaded(&self, id: &ResourceId) -> Option<&Content> {
        self.ready.get(id)
    }

    /// The program's player, while a video is on program.
    pub fn video(&mut self) -> Option<(SceneId, &mut Video)> {
        match &mut self.program {
            Program::Showing {
                scene,
                content: Content::Video(video),
            } => Some((*scene, video)),
            _ => None,
        }
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
            Content::Video(video) => {
                let duration = Some(video.duration()).filter(|d| !d.is_zero());
                engine.report(scene, MediaReport::Timeline { duration });
                video.set_muted(false);
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
        let Some((scene, video)) = self.video() else {
            return;
        };
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

/// Builds a player, paused at the start and muted, so a preload is silent.
/// [`Video::new`] would start playing with sound while it prerolls.
fn open_video(path: &Path) -> Result<Video, String> {
    let fail = |err: &dyn fmt::Display| format!("{}: {err}", path.display());
    let uri = url::Url::from_file_path(path).map_err(|()| fail(&"not an absolute path"))?;
    gst::init().map_err(|err| fail(&err))?;
    let pipeline = gst::parse::launch(&format!(
        "playbin uri=\"{uri}\" mute=true video-sink=\"videoscale ! videoconvert ! \
         appsink name=sink drop=true caps=video/x-raw,format=NV12,pixel-aspect-ratio=1/1\""
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
    video.seek(Duration::ZERO, true).map_err(|err| fail(&err))?;
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
