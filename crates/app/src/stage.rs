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
//! A YouTube video in the plan loads like a local one, from the download
//! cache, and waits for its download when it isn't there yet. One shown
//! outside the plan plays straight from YouTube ([`open_stream`]), and is
//! downloaded only if that fails. An image at a link is downloaded into the
//! cache first, falling back to its [thumbnail](Resource::THUMBNAIL) when the
//! site refuses.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use iced::widget::image::Handle;
use iced_video_player::Video;
use lantern_engine::{
    Engine, FadeTime, Kind, Location, MediaReport, Playback, Resource, ResourceId, SceneId,
    StageState, Timed,
};

use crate::fading::{Fades, Levels, loudness};
use crate::youtube::{Download, Downloads, Stream, YoutubeVideo};

/// What the stage has loaded for one resource.
pub enum Content {
    /// A decoded still image.
    Image(Handle),
    /// A player, paused and muted until it goes on program. `streamed`:
    /// it plays from the net, not from a file.
    Video { player: Video, streamed: bool },
    /// Rendered PDF pages, in order. Never empty.
    Pages(Vec<Handle>),
    /// A player for a sound file: heard, with nothing to show.
    Sound(Sound),
}

impl Content {
    /// A copy to keep while another goes on stage, for what can be shown
    /// twice: a picture or pages, which share their pixels. Not a player.
    fn still(&self) -> Option<Content> {
        match self {
            Content::Image(handle) => Some(Content::Image(handle.clone())),
            Content::Pages(pages) => Some(Content::Pages(pages.clone())),
            Content::Video { .. } | Content::Sound(_) => None,
        }
    }

    /// The memory its pixels take. Pages are read from their files when
    /// drawn, so they take none here.
    fn bytes(&self) -> usize {
        let of = |handle: &Handle| match handle {
            Handle::Rgba { pixels, .. } => pixels.len(),
            Handle::Bytes(_, bytes) => bytes.len(),
            Handle::Path(..) => 0,
        };
        match self {
            Content::Image(handle) => of(handle),
            Content::Pages(pages) => pages.iter().map(of).sum(),
            Content::Video { .. } | Content::Sound(_) => 0,
        }
    }
}

/// The most memory the open plan's pictures are kept in. Past it, the
/// rest load when they're next or shown, as before.
const KEEP_BUDGET: usize = 512 * 1024 * 1024;

/// How many of them load at a time.
const KEEP_AT_ONCE: usize = 2;

/// How the stage presents a resource. Decided from kind and location alone.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Loader {
    Image(PathBuf),
    /// An image at a link, then the links to try if it fails.
    RemoteImage(Vec<String>),
    Video(PathBuf),
    /// A sound file: it plays over what is on stage.
    Audio(PathBuf),
    /// A PDF's pages, or only the one named (counted from 0).
    Pdf(PathBuf, Option<usize>),
    /// A PowerPoint file or its like, shown as the pages of the PDF
    /// LibreOffice makes of it, or only the one named.
    Deck(PathBuf, Option<usize>),
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
    /// A YouTube video's download.
    Youtube(YoutubeVideo),
    /// One of these image links, downloaded or downloadable.
    Images(Vec<String>),
}

/// Showing `resource` plays sound: a video, a sound file.
pub fn has_sound(resource: &Resource) -> bool {
    matches!(
        Loader::for_resource(resource),
        Ok(Some(
            Loader::Video(_) | Loader::Audio(_) | Loader::Youtube(_)
        ))
    )
}

/// What showing `resource` takes, or why the stage can't show it.
pub fn needs(resource: &Resource) -> Result<Needs, String> {
    Ok(match Loader::for_resource(resource)? {
        None => Needs::Nothing,
        Some(
            Loader::Image(path)
            | Loader::Video(path)
            | Loader::Audio(path)
            | Loader::Pdf(path, _)
            | Loader::Deck(path, _),
        ) => Needs::File(path),
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
            Kind::Audio => Ok(Some(Loader::Audio(path))),
            Kind::Document if is_pdf => Ok(Some(Loader::Pdf(path, resource.page_of()))),
            Kind::Presentation if crate::deck::is_deck(&path) => {
                Ok(Some(Loader::Deck(path, resource.page_of())))
            }
            kind => Err(format!("the stage can't show {} files", kind.name())),
        }
    }

    /// Whether it loads something that can be kept and shown again: a
    /// picture or pages, not a player.
    fn is_still(&self) -> bool {
        matches!(
            self,
            Loader::Image(_) | Loader::RemoteImage(_) | Loader::Pdf(..) | Loader::Deck(..)
        )
    }

    /// Blocks: decodes, builds a player, runs `pdftoppm` (after LibreOffice,
    /// for a deck), or waits for or makes a download.
    /// `stream`: a YouTube video that isn't downloaded plays straight from
    /// YouTube, and is downloaded only if that fails.
    fn load(&self, downloads: &Arc<Downloads>, stream: bool) -> Result<Content, String> {
        match self {
            Loader::Image(path) => decode_image(existing(path)?).map(Content::Image),
            Loader::RemoteImage(urls) => {
                fetch_image(urls, &cache_dir().join("images")).map(Content::Image)
            }
            Loader::Video(path) => open_file(path),
            Loader::Audio(path) => Sound::open(existing(path)?).map(Content::Sound),
            Loader::Youtube(video) => {
                if let Some(path) = downloads.cached(video) {
                    return open_file(&path);
                }
                if stream {
                    match streamed(video) {
                        Ok(content) => return Ok(content),
                        Err(err) => crate::log(format!(
                            "stage: can't stream {} ({err}), downloading it",
                            video.id
                        )),
                    }
                }
                match downloads.ready(video) {
                    Ok(path) => open_file(&path),
                    // A live video has nothing to download: it plays as
                    // it comes, in a plan too.
                    Err(_) if downloads.state(&video.id) == Some(Download::Live) => streamed(video),
                    Err(reason) => Err(reason),
                }
            }
            Loader::Pdf(path, page) => render_pdf(existing(path)?, &cache_dir().join("pdf"))
                .and_then(|pages| only(pages, *page))
                .map(Content::Pages),
            Loader::Deck(path, page) => {
                let decks = cache_dir().join("decks");
                let pdf = crate::deck::to_pdf(existing(path)?, &decks)?;
                // Its videos come out now, so one plays the moment it is
                // asked for. One that won't come out is said when it is.
                for attachment in crate::deck::attachments(path).into_iter().flatten() {
                    if let crate::deck::Attachment::Inside(entry) = attachment
                        && let Err(err) = crate::deck::take_out(path, &entry, &decks)
                    {
                        crate::log(format!("stage: {err}"));
                    }
                }
                render_pdf(&pdf, &cache_dir().join("pdf"))
                    .and_then(|pages| only(pages, *page))
                    .map(Content::Pages)
            }
        }
    }
}

/// A player for `video` straight from YouTube.
fn streamed(video: &YoutubeVideo) -> Result<Content, String> {
    let player = crate::youtube::stream(video).and_then(|stream| open_stream(&stream))?;
    crate::log(format!("stage: streaming {}", video.id));
    Ok(Content::Video {
        player,
        streamed: true,
    })
}

/// Sets how loud a player plays, from 0 (silent) to 1 (as recorded): a
/// stream's through its volume element, a file's through its `playbin`.
fn set_volume(player: &mut Video, volume: f64) {
    match player.pipeline().by_name(STREAM_VOLUME) {
        Some(element) => element.set_property("volume", volume),
        None => player.set_volume(volume),
    }
}

/// Lets a player be heard. A file's player is a `playbin`, which has its
/// own mute; a stream's is built here, with a volume element to open.
fn unmute(player: &mut Video) {
    let pipeline = player.pipeline();
    match pipeline.by_name(STREAM_VOLUME) {
        Some(volume) => {
            volume.set_property("mute", false);
            unmute_sinks(&pipeline);
        }
        None => player.set_muted(false),
    }
}

/// Opens the mute of the sound output itself, under a stream's pipeline.
///
/// A `playbin` mutes through its output, which the sound server sees as
/// Lantern's stream being muted, and remembers for the next stream
/// Lantern opens. A stream's pipeline never touched that mute, so it
/// could start with its output muted by a preload of some other video
/// and play in silence.
fn unmute_sinks(pipeline: &gst::Pipeline) {
    let mut elements = pipeline.iterate_recurse();
    while let Ok(Some(element)) = elements.next() {
        let is_sink = element.element_flags().contains(gst::ElementFlags::SINK);
        let has_mute = element
            .find_property("mute")
            .is_some_and(|mute| mute.value_type() == bool::static_type());
        if is_sink && has_mute {
            element.set_property("mute", false);
        }
    }
}

/// The name of the element that mutes a stream's sound.
const STREAM_VOLUME: &str = "lantern-volume";

/// A player for a video straight from the net, paused and silent.
///
/// A video's two streams are plain files over HTTPS. They play through a
/// manifest the relay writes ([`open_indexed`]). Without one, each is
/// decoded by its own branch of one pipeline, so they share a clock and
/// stay together, and each reads through a ring buffer, which is what
/// lets the player seek; there a jump walks the file to its place.
///
/// A live video is its manifest, played by `playbin3` ([`open_live`]): its demuxer keeps
/// picture and sound together, which two branches reading the two lists
/// of pieces would not. It has no length and nowhere to seek.
fn open_stream(stream: &Stream) -> Result<Video, String> {
    let urls = match stream {
        Stream::Live(manifest) => {
            let uri = url::Url::parse(manifest).map_err(|err| format!("{manifest}: {err}"))?;
            return open_live(&uri);
        }
        Stream::Pair {
            video,
            audio,
            seconds,
        } => {
            let files = vec![video.url.clone(), audio.url.clone()];
            let Some(manifest) = crate::relay::manifest(video, audio, *seconds) else {
                return open_stream(&Stream::Files(files));
            };
            return open_indexed(&manifest).or_else(|err| {
                crate::log(format!("stage: the manifest wouldn't play ({err})"));
                open_stream(&Stream::Files(files))
            });
        }
        Stream::Files(urls) => urls,
    };
    // Read through the relay, which asks YouTube for a piece at a time:
    // read in one go, YouTube slows a file to twice the speed it plays.
    let urls: Vec<String> = urls
        .iter()
        .map(|url| crate::relay::through(url).unwrap_or_else(|| url.clone()))
        .collect();
    let (video, audio) = match urls.as_slice() {
        [both] => {
            let uri = url::Url::parse(both).map_err(|err| format!("{both}: {err}"))?;
            return open_video(&uri);
        }
        [video, audio] => (video, audio),
        _ => return Err("no stream to play".into()),
    };
    let fail = |err: &dyn fmt::Display| format!("the stream: {err}");
    gst::init().map_err(|err| fail(&err))?;
    let pipeline = gst::parse::launch(&format!(
        "uridecodebin uri=\"{video}\" ring-buffer-max-size=67108864 ! queue ! \
         videoscale add-borders=true ! videoconvert ! appsink name=sink drop=true \
         caps=video/x-raw,format=NV12,pixel-aspect-ratio=1/1 \
         uridecodebin uri=\"{audio}\" ring-buffer-max-size=16777216 ! queue ! \
         audioconvert ! audioresample ! volume name={STREAM_VOLUME} mute=true ! autoaudiosink"
    ))
    .map_err(|err| fail(&err))?
    .downcast::<gst::Pipeline>()
    .map_err(|_| fail(&"not a pipeline"))?;
    let sink = pipeline
        .by_name("sink")
        .and_then(|sink| sink.downcast::<gstreamer_app::AppSink>().ok())
        .ok_or_else(|| fail(&"no video sink"))?;
    wait_to_start(&pipeline).map_err(|err| fail(&err))?;
    let mut player = Video::from_gst_pipeline(pipeline, sink, None).map_err(|err| fail(&err))?;
    player.set_paused(true);
    player
        .seek(Duration::ZERO, true)
        .map_err(|err| fail(&err))?;
    Ok(player)
}

/// A player for a video's two files through the relay's manifest, at its
/// start. `playbin3` plays it: its demuxer reads each file's index, so a
/// jump goes straight to its place.
fn open_indexed(manifest: &str) -> Result<Video, String> {
    let uri = url::Url::parse(manifest).map_err(|err| format!("{manifest}: {err}"))?;
    let mut player = open_playbin("playbin3", &uri, "", true)?;
    player
        .seek(Duration::ZERO, true)
        .map_err(|err| format!("the stream: {err}"))?;
    Ok(player)
}

/// How long a stream may take to show its first picture.
const STREAM_START: Duration = Duration::from_secs(20);

/// Blocks until `pipeline` has its first picture and sound, paused, for at
/// most [`STREAM_START`]. The player gives a pipeline five seconds to get
/// there and then fails, which a slow network doesn't always manage; one
/// that is there already passes at once.
fn wait_to_start(pipeline: &gst::Pipeline) -> Result<(), String> {
    // Why it stopped, when the pipeline said: "Forbidden", "Not Found".
    let stop = |reason: &str| {
        let said = pipeline
            .bus()
            .and_then(|bus| bus.pop_filtered(&[gst::MessageType::Error]));
        let _ = pipeline.set_state(gst::State::Null);
        Err(match said.as_ref().map(|said| said.view()) {
            Some(gst::MessageView::Error(err)) => {
                format!("{reason} ({})", err.error().message().trim_end_matches('.'))
            }
            _ => reason.to_string(),
        })
    };
    if pipeline.set_state(gst::State::Paused).is_err() {
        return stop("it wouldn't start");
    }
    let wait = gst::ClockTime::from_nseconds(STREAM_START.as_nanos() as u64);
    match pipeline.state(wait) {
        (Ok(gst::StateChangeSuccess::Async), ..) => stop("it took too long to start"),
        (Ok(_), ..) => Ok(()),
        (Err(_), ..) => stop("it wouldn't start"),
    }
}

/// A player for a sound file, paused and silent until it goes on program.
///
/// A sound has no picture, and the video player is built around one, so
/// this is its own: a `playbin` whose pictures (an album's cover) go
/// nowhere.
pub struct Sound {
    player: gst::Element,
    duration: Duration,
}

/// How long a sound file may take to be ready to play.
const SOUND_START: gst::ClockTime = gst::ClockTime::from_seconds(15);

impl Sound {
    fn open(path: &Path) -> Result<Sound, String> {
        let fail = |err: &dyn fmt::Display| format!("{}: {err}", path.display());
        gst::init().map_err(|err| fail(&err))?;
        let uri = url::Url::from_file_path(path).map_err(|()| fail(&"not an absolute path"))?;
        let nowhere = gst::ElementFactory::make("fakesink")
            .build()
            .map_err(|err| fail(&err))?;
        let player = gst::ElementFactory::make("playbin")
            .property("uri", uri.as_str())
            .property("mute", true)
            .property("video-sink", &nowhere)
            .build()
            .map_err(|err| fail(&err))?;
        let mut sound = Sound {
            player,
            duration: Duration::ZERO,
        };
        sound
            .player
            .set_state(gst::State::Paused)
            .map_err(|err| fail(&err))?;
        // Paused is reached once the file is read far enough to play.
        if sound.player.state(SOUND_START).0.is_err() {
            let why = match sound.event() {
                Some(Err(why)) => why,
                _ => "it can't be played".to_string(),
            };
            return Err(fail(&why));
        }
        sound.duration = sound
            .player
            .query_duration::<gst::ClockTime>()
            .map_or(Duration::ZERO, |length| {
                Duration::from_nanos(length.nseconds())
            });
        Ok(sound)
    }

    /// Its length; zero while it can't say. A file that didn't say as
    /// it opened is asked again.
    pub fn duration(&self) -> Duration {
        if !self.duration.is_zero() {
            return self.duration;
        }
        self.player
            .query_duration::<gst::ClockTime>()
            .map_or(Duration::ZERO, |length| {
                Duration::from_nanos(length.nseconds())
            })
    }

    fn set_paused(&self, paused: bool) {
        let state = if paused {
            gst::State::Paused
        } else {
            gst::State::Playing
        };
        if let Err(err) = self.player.set_state(state) {
            crate::log(format!("stage: sound: {err}"));
        }
    }

    fn unmute(&self) {
        self.player.set_property("mute", false);
    }

    fn set_volume(&self, volume: f64) {
        self.player.set_property("volume", volume);
    }

    /// Where it is, or `None` while it is settling after a jump.
    fn position(&self) -> Option<Duration> {
        let settling = matches!(
            self.player.state(gst::ClockTime::ZERO).0,
            Ok(gst::StateChangeSuccess::Async)
        );
        if settling {
            return None;
        }
        self.player
            .query_position::<gst::ClockTime>()
            .map(|at| Duration::from_nanos(at.nseconds()))
    }

    fn seek(&self, position: Duration) -> Result<(), String> {
        // An end said before the jump is no longer true.
        if let Some(bus) = self.player.bus() {
            while bus.pop_filtered(&[gst::MessageType::Eos]).is_some() {}
        }
        self.player
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::ClockTime::from_nseconds(position.as_nanos() as u64),
            )
            .map_err(|err| err.to_string())
    }

    /// What it has said since it was last asked: that it reached its end
    /// (`Ok`), or why it stopped (`Err`).
    fn event(&self) -> Option<Result<(), String>> {
        let message = self
            .player
            .bus()?
            .pop_filtered(&[gst::MessageType::Eos, gst::MessageType::Error])?;
        Some(match message.view() {
            gst::MessageView::Error(err) => Err(err.error().to_string()),
            _ => Ok(()),
        })
    }
}

impl Drop for Sound {
    fn drop(&mut self) {
        let _ = self.player.set_state(gst::State::Null);
    }
}

/// Where `player` is, or `None` while it can't say. A player in the middle
/// of a seek answers nothing, which [`Video::position`] turns into zero:
/// on a stream that lasts seconds, and the time on the dashboard fell to
/// 0:00 until the picture came back.
pub fn position(player: &Video) -> Option<Duration> {
    let pipeline = player.pipeline();
    let settling = matches!(
        pipeline.state(gst::ClockTime::ZERO).0,
        Ok(gst::StateChangeSuccess::Async)
    );
    if settling {
        return None;
    }
    pipeline
        .query_position::<gst::ClockTime>()
        .map(|at| Duration::from_nanos(at.nseconds()))
}

/// How far from its length a stream may say it ended and be believed:
/// the length `yt-dlp` gives is to the second, and the last frame isn't
/// the last instant.
const END_MARGIN: Duration = Duration::from_secs(5);

/// Whether a video of `length` that says it ended `at` stopped short. One
/// with no length is live, and ends when it ends.
fn stopped_short(at: Duration, length: Duration) -> bool {
    !length.is_zero() && at + END_MARGIN < length
}

/// What the operator reads when a playing video stops on `why`.
fn broke_reason(streamed: bool, why: &str) -> String {
    let why = why.trim_end_matches('.');
    if streamed {
        format!("the stream from YouTube stopped, is the network up? ({why})")
    } else {
        format!("the video stopped playing ({why})")
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
        streamed: false,
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

/// What the stage wants loaded right now, shared with the loads that wait
/// before they start ([`worth_loading`]).
#[derive(Default)]
struct Wanted {
    /// What the last [`preload`](Stage::preload) asked for.
    upcoming: HashSet<ResourceId>,
    /// What the program is waiting for.
    program: Option<ResourceId>,
}

/// How long a YouTube video must stay lined up before Lantern asks
/// YouTube for it. Going down a list of results lines each one up in
/// passing, and asking for each started a `yt-dlp` of several seconds for
/// every result passed, none of them stopped.
const DWELL: Duration = Duration::from_millis(400);

/// What a load answers when its video was passed over before it started.
const PASSED_OVER: &str = "passed over";

/// Waits `dwell`, then says whether `id` is still lined up. The program
/// waiting for it ends the wait at once.
fn worth_loading(wanted: &Mutex<Wanted>, id: &ResourceId, dwell: Duration) -> bool {
    let until = Instant::now() + dwell;
    loop {
        {
            let wanted = wanted
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if wanted.program.as_ref() == Some(id) {
                return true;
            }
            if Instant::now() >= until {
                return wanted.upcoming.contains(id);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Starts loading on a worker thread. `dwell`: a YouTube video that isn't
/// on disk waits [`DWELL`] first, and isn't loaded if by then it is no
/// longer wanted.
fn spawn(
    resource: ResourceId,
    loader: Loader,
    downloads: Arc<Downloads>,
    stream: bool,
    dwell: Option<Arc<Mutex<Wanted>>>,
) -> iced::Task<Arc<Loaded>> {
    iced::Task::future(async move {
        let (done, result) = iced::futures::channel::oneshot::channel();
        let id = resource.clone();
        std::thread::spawn(move || {
            let passed_over = match (&loader, &dwell) {
                (Loader::Youtube(video), Some(wanted)) => {
                    downloads.cached(video).is_none() && !worth_loading(wanted, &id, DWELL)
                }
                _ => false,
            };
            let _ = done.send(if passed_over {
                Err(PASSED_OVER.to_string())
            } else {
                loader.load(&downloads, stream)
            });
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

/// The sound playing apart from the program ([`StageState::bed`]).
struct Bed {
    scene: SceneId,
    /// What it is loaded as: apart from the same file loading for the
    /// program or a preload.
    key: ResourceId,
    /// Its player once loaded, and when it began to play.
    playing: Option<(Sound, Instant)>,
    /// How long it takes to come up from nothing.
    fade_in: Option<Duration>,
    /// It fades on its own ([`lantern_engine::Step::sound_fade`]), not
    /// with what is seen.
    alone: bool,
}

/// A sound that played apart from the program, going out: from `from`
/// of the way up, over `over` from `since`.
struct Going {
    sound: Sound,
    from: f32,
    since: Instant,
    over: Duration,
}

/// How far up the background track is while it plays under another
/// sound.
const LOWERED: f32 = 0.55;

/// How long the background track takes to go down under another sound,
/// or back up.
const DUCK: Duration = Duration::from_millis(800);

/// Where a sound apart from the program plays: a step's sound in the
/// bed ([`StageState::bed`]), or the background track
/// ([`StageState::track`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Apart {
    Bed = 0,
    Track = 1,
}

impl Apart {
    const BOTH: [Apart; 2] = [Apart::Bed, Apart::Track];

    fn of(self, stage: &StageState) -> Option<&lantern_engine::Scene> {
        match self {
            Apart::Bed => stage.bed.as_ref(),
            Apart::Track => stage.track.as_ref(),
        }
    }

    /// What it is called, and what its file is loaded as.
    fn part(self) -> &'static str {
        match self {
            Apart::Bed => "bed",
            Apart::Track => "track",
        }
    }
}

/// The background track going down under another sound, or back up:
/// from `from` of the way up, since `since`.
#[derive(Debug, Clone, Copy)]
struct Duck {
    from: f32,
    down: bool,
    since: Instant,
}

impl Duck {
    fn to(&self) -> f32 {
        match self.down {
            true => LOWERED,
            false => 1.0,
        }
    }

    fn level(&self, now: Instant) -> f32 {
        let gone = now.saturating_duration_since(self.since).as_secs_f32() / DUCK.as_secs_f32();
        self.from + (self.to() - self.from) * gone.min(1.0)
    }

    fn moving(&self, now: Instant) -> bool {
        self.from != self.to() && now.saturating_duration_since(self.since) < DUCK
    }
}

/// How far up something is that comes up over `fade_in` and began at
/// `since`.
fn come_up(fade_in: Option<Duration>, since: Instant, now: Instant) -> f32 {
    match fade_in {
        Some(over) if !over.is_zero() => {
            (now.saturating_duration_since(since).as_secs_f32() / over.as_secs_f32()).min(1.0)
        }
        _ => 1.0,
    }
}

/// The renderer's state. The engine's mirror ([`StageState`]) stays the
/// authority on what is shown; this only holds what it takes to draw it.
pub struct Stage {
    /// The sounds playing apart from the program, by [`Apart`], and
    /// those going out.
    apart: [Option<Bed>; 2],
    going: Vec<Going>,
    /// The background track going down under another sound, or back up.
    duck: Duck,
    program: Program,
    /// What is loaded for the picture a sound plays over
    /// ([`StageState::under`]): it was the program when the sound went on.
    under: Option<(SceneId, Content)>,
    /// Loaded ahead of time for the preview or the next plan step.
    ready: HashMap<ResourceId, Content>,
    /// Loads in flight, for the program or a preload.
    loading: HashSet<ResourceId>,
    /// What the last [`preload`](Stage::preload) asked for.
    upcoming: HashSet<ResourceId>,
    /// The same, and what the program waits for, where the loads that
    /// wait before starting can see it.
    wanted: Arc<Mutex<Wanted>>,
    /// The loads that are waiting, or were, with what they load: one
    /// passed over and then wanted after all is started again.
    dwelling: HashMap<ResourceId, Loader>,
    /// The open plan's pictures and pages, loaded once and kept while the
    /// plan is open, so they show at once in any order.
    kept: HashMap<ResourceId, Content>,
    /// What the last [`keep`](Stage::keep) asked for.
    keeping: HashSet<ResourceId>,
    /// Those of them that won't be kept: they failed to load, or there was
    /// no room left. Not tried again while they stay in the plan.
    unkept: HashSet<ResourceId>,
    /// Those still to load, in order, and the few loading now: a plan of
    /// forty pictures mustn't decode them all at once.
    to_keep: VecDeque<(ResourceId, Loader)>,
    keep_loading: HashSet<ResourceId>,
    /// The YouTube videos the open plan shows (its steps and its standby
    /// shelf), by id: these are downloaded. Any other is streamed, when
    /// `stream_others` says so.
    planned: HashSet<String>,
    stream_others: bool,
    /// The real length of every video loaded so far, for timing the plan.
    lengths: HashMap<ResourceId, Duration>,
    downloads: Arc<Downloads>,
    /// How far up, or down, what is on stage has faded.
    fades: Fades,
    /// The levels last played and drawn. A fade ends between two ticks,
    /// and goes on until its last level has been.
    played: Levels,
}

impl Stage {
    pub fn new(downloads: Arc<Downloads>) -> Self {
        Stage {
            apart: [None, None],
            going: Vec::new(),
            duck: Duck {
                from: 1.0,
                down: false,
                since: Instant::now(),
            },
            downloads,
            program: Program::Bare,
            under: None,
            ready: HashMap::new(),
            loading: HashSet::new(),
            upcoming: HashSet::new(),
            kept: HashMap::new(),
            keeping: HashSet::new(),
            unkept: HashSet::new(),
            planned: HashSet::new(),
            stream_others: false,
            to_keep: VecDeque::new(),
            keep_loading: HashSet::new(),
            lengths: HashMap::new(),
            wanted: Arc::default(),
            dwelling: HashMap::new(),
            fades: Fades::default(),
            played: Levels::FULL,
        }
    }

    /// Follows the stage's fades: what left it, and a fade out begun or
    /// called off, which plays what is on as loud as it was.
    pub fn sync_fades(&mut self, stage: &StageState) {
        let now = Instant::now();
        self.fades.sync(stage, now);
        self.play_levels(stage, now);
    }

    /// Follows a sound playing apart from the program, `which` of them:
    /// the one it takes the place of goes out over `fade_out`, or at
    /// once, and a new one starts loading. One that can't be played says
    /// why.
    pub fn bed_changed(
        &mut self,
        stage: &StageState,
        which: Apart,
        fade_out: Option<FadeTime>,
        engine: &Engine,
    ) -> iced::Task<Arc<Loaded>> {
        let wanted = which.of(stage);
        let slot = which as usize;
        if self.apart[slot].as_ref().map(|bed| bed.scene) == wanted.map(|scene| scene.id) {
            return iced::Task::none();
        }
        let now = Instant::now();
        if let Some(Bed {
            playing: Some((sound, since)),
            fade_in,
            alone,
            ..
        }) = self.apart[slot].take()
        {
            let from = come_up(fade_in, since, now)
                * self.with_seen(alone, stage, now)
                * self.ducked(which, now);
            if let Some(over) = fade_out {
                self.going.push(Going {
                    sound,
                    from,
                    since: now,
                    over: over.duration(),
                });
            }
        }
        let Some(scene) = wanted else {
            crate::log(format!("stage: no {} plays", which.part()));
            return iced::Task::none();
        };
        let path = match &scene.resource.location {
            Location::File(path) => path.clone(),
            _ => {
                engine.report(
                    scene.id,
                    MediaReport::Failed {
                        reason: "only a file can play apart from the program".into(),
                    },
                );
                return iced::Task::none();
            }
        };
        let key = scene.resource.id.part(which.part());
        // A background track goes on from step to step, whatever fades;
        // a step's sound apart fades with what it shows unless it fades
        // on its own.
        let alone = match which {
            Apart::Track => true,
            Apart::Bed => engine
                .plays_apart()
                .filter(|(playing, _)| *playing == scene.id)
                .and_then(|(_, owner)| engine.plan().get(owner).map(|step| step.sound_fade))
                .is_some_and(|fade| fade.is_some()),
        };
        self.apart[slot] = Some(Bed {
            scene: scene.id,
            key: key.clone(),
            playing: None,
            fade_in: scene.fade_in.map(FadeTime::duration),
            alone,
        });
        crate::log(format!("stage: {} plays as the {}", scene.id, which.part()));
        spawn(
            key,
            Loader::Audio(path),
            self.downloads.clone(),
            false,
            None,
        )
    }

    /// Plays the sounds apart from the program, and those going out, as
    /// loud as their fades say at `now`; one gone out is let go. While
    /// what is on fades out, a step's sound apart fades with it. The
    /// background track goes down under another sound, and back up,
    /// over [`DUCK`].
    fn play_bed(&mut self, stage: &StageState, now: Instant) {
        if stage.lowered != self.duck.down {
            self.duck = Duck {
                from: self.duck.level(now),
                down: stage.lowered,
                since: now,
            };
        }
        for which in Apart::BOTH {
            if let Some(Bed {
                playing: Some((sound, since)),
                fade_in,
                alone,
                ..
            }) = &self.apart[which as usize]
            {
                let with = self.with_seen(*alone, stage, now) * self.ducked(which, now);
                sound.set_volume(loudness(come_up(*fade_in, *since, now) * with));
            }
        }
        self.going.retain(|going| {
            let gone = now.saturating_duration_since(going.since).as_secs_f32()
                / going.over.as_secs_f32().max(f32::EPSILON);
            going.sound.set_volume(loudness(going.from * (1.0 - gone)));
            gone < 1.0
        });
    }

    /// How far up what is seen lets a sound apart from the program be:
    /// as far as it is faded, unless the sound fades on its own.
    fn with_seen(&self, alone: bool, stage: &StageState, now: Instant) -> f32 {
        match alone {
            true => 1.0,
            false => self.fades.levels(stage, now).sound,
        }
    }

    /// How far down `which` is gone under another sound: the background
    /// track only.
    fn ducked(&self, which: Apart, now: Instant) -> f32 {
        match which {
            Apart::Bed => 1.0,
            Apart::Track => self.duck.level(now),
        }
    }

    /// Whether a sound apart from the program is coming up, going down or
    /// back up, or one is going out.
    fn bed_moving(&self, now: Instant) -> bool {
        !self.going.is_empty()
            || self.duck.moving(now)
            || self.apart.iter().flatten().any(|bed| {
                bed.playing
                    .as_ref()
                    .is_some_and(|(_, since)| come_up(bed.fade_in, *since, now) < 1.0)
            })
    }

    /// The sounds apart from the program that are playing.
    fn beds(&self) -> impl Iterator<Item = (SceneId, &Sound)> {
        self.apart
            .iter()
            .flatten()
            .filter_map(|bed| Some((bed.scene, &bed.playing.as_ref()?.0)))
    }

    /// Where each sound apart from the program is now.
    pub fn bed_playheads(&self) -> Vec<(SceneId, Duration)> {
        self.beds()
            .filter_map(|(scene, sound)| Some((scene, sound.position()?)))
            .collect()
    }

    /// How long each sound apart from the program is, once its player
    /// knows: a file may not say so as it starts.
    pub fn bed_lengths(&self) -> Vec<(SceneId, Duration)> {
        self.beds()
            .map(|(scene, sound)| (scene, sound.duration()))
            .filter(|(_, length)| !length.is_zero())
            .collect()
    }

    /// What each sound apart from the program has said since it was last
    /// asked: that it reached its end, or why it stopped.
    pub fn bed_events(&self) -> Vec<(SceneId, Result<(), String>)> {
        self.beds()
            .filter_map(|(scene, sound)| Some((scene, sound.event()?)))
            .collect()
    }

    /// A sound apart from the program, loaded: it plays from silence or
    /// at once. `true` when `loaded` was one.
    fn bed_loaded(&mut self, loaded: &Loaded, stage: &StageState, engine: &Engine) -> bool {
        let Some(which) = Apart::BOTH.into_iter().find(|which| {
            self.apart[*which as usize]
                .as_ref()
                .is_some_and(|bed| bed.key == loaded.resource && bed.playing.is_none())
        }) else {
            return false;
        };
        self.loading.remove(&loaded.resource);
        let Some(result) = loaded.result.lock().ok().and_then(|mut slot| slot.take()) else {
            return true;
        };
        let now = Instant::now();
        let ducked = self.ducked(which, now);
        let seen = self.fades.levels(stage, now).sound;
        let Some(bed) = self.apart[which as usize].as_mut() else {
            return true;
        };
        match result {
            Ok(Content::Sound(sound)) => {
                let seen = if bed.alone { 1.0 } else { seen };
                let level = come_up(bed.fade_in, now, now) * seen * ducked;
                sound.set_volume(loudness(level));
                sound.unmute();
                sound.set_paused(false);
                let duration = Some(sound.duration()).filter(|d| !d.is_zero());
                engine.report(bed.scene, MediaReport::Timeline { duration });
                bed.playing = Some((sound, now));
                crate::log(format!(
                    "stage: {} playing as the {}",
                    bed.scene,
                    which.part()
                ));
            }
            Ok(_) => {}
            Err(reason) => {
                crate::log(format!("stage: {} can't play apart: {reason}", bed.scene));
                engine.report(bed.scene, MediaReport::Failed { reason });
            }
        }
        true
    }

    /// How much of what is on gets through now, as it fades.
    pub fn levels(&self, stage: &StageState) -> Levels {
        self.fades.levels(stage, Instant::now())
    }

    /// Whether something on stage is fading up or out now, or its last
    /// level is still to be played and drawn.
    pub fn fading(&self, stage: &StageState) -> bool {
        let now = Instant::now();
        self.fades.moving(stage, now)
            || self.played != self.fades.levels(stage, now)
            || self.bed_moving(now)
    }

    /// Plays what is on as loud as its fade says, and once what was
    /// fading out has, tells the engine, which goes on. Returns whether
    /// the engine took it.
    pub fn fade(&mut self, stage: &StageState, engine: &Engine) -> bool {
        let now = Instant::now();
        self.fades.sync(stage, now);
        self.play_levels(stage, now);
        self.fades
            .faded(stage, now)
            .is_some_and(|program| engine.faded(program))
    }

    /// Plays the program as loud as its fade says at `now`, and keeps the
    /// levels as played.
    fn play_levels(&mut self, stage: &StageState, now: Instant) {
        self.played = self.fades.levels(stage, now);
        // Silenced while something plays in its place.
        self.set_loudness(match stage.muted {
            true => 0.0,
            false => self.played.sound,
        });
        self.play_bed(stage, now);
    }

    /// Sets how loud the program's player plays, `level` of the way up.
    fn set_loudness(&mut self, level: f32) {
        match &mut self.program {
            Program::Showing {
                content: Content::Video { player, .. },
                ..
            } => set_volume(player, loudness(level)),
            Program::Showing {
                content: Content::Sound(sound),
                ..
            } => sound.set_volume(loudness(level)),
            _ => {}
        }
    }

    /// Tells the waiting loads what is wanted now.
    fn sync_wanted(&self) {
        let mut wanted = self
            .wanted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        wanted.upcoming = self.upcoming.clone();
        wanted.program = match &self.program {
            Program::Loading { resource, .. } => Some(resource.clone()),
            _ => None,
        };
    }

    /// Says which YouTube videos the open plan shows, and whether the
    /// others are streamed instead of downloaded.
    pub fn plan_videos(&mut self, planned: HashSet<String>, stream_others: bool) {
        self.planned = planned;
        self.stream_others = stream_others;
    }

    /// Whether what `loader` loads is streamed if it can be: a YouTube
    /// video the plan doesn't show.
    fn streams(&self, loader: &Loader) -> bool {
        match loader {
            Loader::Youtube(video) => self.stream_others && !self.planned.contains(&video.id),
            _ => false,
        }
    }

    /// The content for the program scene, or for the picture under a
    /// sound, once it has loaded.
    /// A picture or a page loaded already, by what it is: kept with the
    /// open plan, or loaded ahead of time.
    pub fn still(&self, id: &ResourceId) -> Option<&Content> {
        self.kept.get(id).or_else(|| self.ready.get(id))
    }

    pub fn content(&self, scene: SceneId) -> Option<&Content> {
        match (&self.program, &self.under) {
            (
                Program::Showing {
                    scene: shown,
                    content,
                },
                _,
            ) if *shown == scene => Some(content),
            (_, Some((under, content))) if *under == scene => Some(content),
            _ => None,
        }
    }

    /// The program's player, while a sound is on program.
    fn sound(&self) -> Option<(SceneId, &Sound)> {
        match &self.program {
            Program::Showing {
                scene,
                content: Content::Sound(sound),
            } => Some((*scene, sound)),
            _ => None,
        }
    }

    /// Where what is playing on program is, for the dashboard's time.
    /// Nothing while a player is seeking: the time stays where the seek
    /// put it.
    pub fn playhead(&mut self) -> Option<(SceneId, Duration)> {
        if let Some((scene, sound)) = self.sound() {
            return Some((scene, sound.position()?));
        }
        let (scene, video) = self.video()?;
        Some((scene, position(video)?))
    }

    /// What the sound on program has said since it was last asked: that
    /// it reached its end, or why it stopped. A video says these through
    /// its picture; a sound has to be asked.
    pub fn sound_event(&self) -> Option<(SceneId, Result<(), String>)> {
        let (scene, sound) = self.sound()?;
        Some((scene, sound.event()?))
    }

    /// A video's real length, once it has loaded.
    pub fn length(&self, id: &ResourceId) -> Option<Duration> {
        self.lengths.get(id).copied()
    }

    /// What a preload has finished loading for `id`, to show it.
    pub fn preloaded(&self, id: &ResourceId) -> Option<&Content> {
        // Or one of the plan's pictures or pages, kept loaded.
        self.ready.get(id).or_else(|| self.kept.get(id))
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
        self.fades.sync(stage, Instant::now());
        let task = self.show(stage, engine);
        self.ready.retain(|id, _| self.upcoming.contains(id));
        self.sync_wanted();
        task
    }

    fn show(&mut self, stage: &StageState, engine: &Engine) -> iced::Task<Arc<Loaded>> {
        // A sound went on over what was on program: that picture stays,
        // and what was loaded for it with it.
        let was = std::mem::replace(&mut self.program, Program::Bare);
        // A sound came off the picture it played over: the picture is the
        // program again, with what was loaded for it.
        let lifted = stage.program.as_ref().map(|scene| scene.id);
        if let Some((scene, content)) = self.under.take_if(|(id, _)| Some(*id) == lifted) {
            self.install(scene, Ok(content), stage, engine);
            return iced::Task::none();
        }
        self.under = match (&stage.under, was, self.under.take()) {
            (Some(under), Program::Showing { scene, content }, _) if scene == under.id => {
                Some((scene, content))
            }
            (Some(under), _, Some(kept)) if kept.0 == under.id => Some(kept),
            _ => None,
        };
        let Some(scene) = &stage.program else {
            return iced::Task::none();
        };
        let loader = match Loader::for_resource(&scene.resource) {
            Ok(Some(loader)) => loader,
            // Drawn from the scene itself: ready as it goes on.
            Ok(None) => {
                self.fades.up(scene.id, Instant::now());
                return iced::Task::none();
            }
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
        if let Some(content) = self.kept.get(&resource).and_then(Content::still) {
            crate::log(format!("stage: {} was kept loaded", scene.id));
            self.install(scene.id, Ok(content), stage, engine);
            return iced::Task::none();
        }
        self.program = Program::Loading {
            scene: scene.id,
            resource: resource.clone(),
        };
        if self.loading.insert(resource.clone()) {
            let stream = self.streams(&loader);
            spawn(resource, loader, self.downloads.clone(), stream, None)
        } else {
            iced::Task::none()
        }
    }

    /// A load finished: it goes on program if the program is waiting for
    /// it, into `ready` if it is still upcoming, and is dropped otherwise.
    ///
    /// A load that was passed over before it started, and is wanted after
    /// all, starts again: the task returned.
    pub fn loaded(
        &mut self,
        loaded: &Loaded,
        stage: &StageState,
        engine: &Engine,
    ) -> iced::Task<Arc<Loaded>> {
        if self.bed_loaded(loaded, stage, engine) {
            return iced::Task::none();
        }
        self.loading.remove(&loaded.resource);
        self.keep_loading.remove(&loaded.resource);
        let dwelt = self.dwelling.remove(&loaded.resource);
        let Some(result) = loaded.result.lock().ok().and_then(|mut slot| slot.take()) else {
            return iced::Task::none();
        };
        if matches!(&result, Err(why) if why == PASSED_OVER) {
            let waited_for = matches!(&self.program, Program::Loading { resource, .. } if *resource == loaded.resource);
            let Some(loader) =
                dwelt.filter(|_| waited_for || self.upcoming.contains(&loaded.resource))
            else {
                return iced::Task::none();
            };
            let id = loaded.resource.clone();
            self.loading.insert(id.clone());
            let stream = self.streams(&loader);
            return spawn(id, loader, self.downloads.clone(), stream, None);
        }
        let length = match &result {
            Ok(Content::Video { player, .. }) => player.duration(),
            Ok(Content::Sound(sound)) => sound.duration(),
            _ => Duration::ZERO,
        };
        if !length.is_zero() {
            self.lengths.insert(loaded.resource.clone(), length);
        }
        // The picture a sound was started over, which was still loading
        // when the sound went on: it is shown under it after all.
        if let Some(under) = &stage.under
            && under.resource.id == loaded.resource
            && self.under.is_none()
            && result
                .as_ref()
                .is_ok_and(|content| content.still().is_some())
        {
            self.under = result.ok().map(|content| (under.id, content));
            return iced::Task::none();
        }
        // One of the plan's pictures: a copy stays for the next time.
        if self.keeping.contains(&loaded.resource) {
            let held: usize = self.kept.values().map(Content::bytes).sum();
            match result.as_ref().ok().and_then(Content::still) {
                Some(copy) if held + copy.bytes() <= KEEP_BUDGET => {
                    let held = (held + copy.bytes()) / (1024 * 1024);
                    self.kept.insert(loaded.resource.clone(), copy);
                    crate::log(format!(
                        "stage: keeping {} of the plan's pictures loaded ({held} MB)",
                        self.kept.len()
                    ));
                }
                _ => {
                    self.unkept.insert(loaded.resource.clone());
                }
            }
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
        iced::Task::none()
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
        // Ready to show: one that fades in starts from silence.
        let now = Instant::now();
        self.fades.up(scene, now);
        // Silenced while something plays in its place.
        let volume = match stage.muted {
            true => 0.0,
            false => loudness(self.fades.levels(stage, now).sound),
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
                set_volume(player, volume);
                unmute(player);
            }
            Content::Sound(sound) => {
                let duration = Some(sound.duration()).filter(|d| !d.is_zero());
                engine.report(scene, MediaReport::Timeline { duration });
                sound.set_volume(volume);
                sound.unmute();
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

    /// The program's player stopped on an error: the scene has failed and
    /// says why, and the player is let go. A stream whose network went
    /// away stops like this, and then says it ended; the engine takes no
    /// end from a scene that failed.
    pub fn broke(&mut self, scene: SceneId, why: &str, engine: &Engine) {
        let streamed = match &self.program {
            Program::Showing {
                scene: shown,
                content: Content::Video { streamed, .. },
            } if *shown == scene => *streamed,
            Program::Showing {
                scene: shown,
                content: Content::Sound(_),
            } if *shown == scene => false,
            _ => return,
        };
        let reason = broke_reason(streamed, why);
        crate::log(format!("stage: {scene} failed: {reason}"));
        engine.report(scene, MediaReport::Failed { reason });
        self.program = Program::Bare;
    }

    /// The program's player says it reached its end. A stream that says
    /// so well short of its length didn't: its network went away, and the
    /// player that takes a manifest ends there without an error. That is
    /// a failure ([`broke`](Stage::broke)), not an end.
    pub fn ended(&mut self, scene: SceneId, engine: &Engine) {
        let short = match &self.program {
            Program::Showing {
                scene: shown,
                content:
                    Content::Video {
                        player,
                        streamed: true,
                    },
            } if *shown == scene => position(player)
                .filter(|at| stopped_short(*at, player.duration()))
                .map(|at| {
                    format!(
                        "it ended at {} of {}",
                        crate::length::clock(at),
                        crate::length::clock(player.duration())
                    )
                }),
            _ => None,
        };
        match short {
            Some(why) => self.broke(scene, &why, engine),
            None => {
                engine.report(scene, MediaReport::Ended);
            }
        }
    }

    /// Plays or pauses the player to match the engine.
    pub fn sync_playback(&mut self, timed: &Timed, engine: &Engine) {
        if let Some((scene, sound)) = self.sound() {
            let paused = timed.playback != Playback::Playing;
            sound.set_paused(paused);
            if paused && let Some(position) = sound.position() {
                engine.report(scene, MediaReport::Position(position));
            }
            return;
        }
        let Some((scene, video)) = self.video() else {
            return;
        };
        let paused = timed.playback != Playback::Playing;
        // Told every time, not only when the player says it differs: one
        // just opened is still on its way to paused and says "playing"
        // until it gets there, and then stays paused.
        video.set_paused(paused);
        if paused && let Some(position) = position(video) {
            engine.report(scene, MediaReport::Position(position));
        }
    }

    /// Jumps the player and reports where it landed.
    pub fn seek(&mut self, position: Duration, engine: &Engine) {
        if let Some((scene, sound)) = self.sound() {
            if let Err(err) = sound.seek(position) {
                crate::log(format!("stage: seek failed: {err}"));
            }
            engine.report(scene, MediaReport::Position(position));
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
            // Restarting plays for a moment before it seeks, and a pipeline
            // set playing at its end says "ended" once more. Left on the
            // bus, that would end a stream again the moment it restarted.
            if let Some(bus) = video.pipeline().bus() {
                while bus.pop_filtered(&[gst::MessageType::Eos]).is_some() {}
            }
        }
        if let Err(err) = video.seek(position, true) {
            crate::log(format!("stage: seek failed: {err}"));
        }
        engine.report(scene, MediaReport::Position(position));
    }

    /// Loads the open plan's pictures and PDF pages and keeps them while
    /// they stay in the plan, so any of them shows at once: `resources` is
    /// everything the plan may show, in the order to load it. Videos aren't
    /// kept (a player each is too much); the next one is still preloaded.
    /// What left the plan is let go.
    /// Lets go of what is kept of the pages of the file `file` is: they
    /// load again, from the file as it is now.
    pub fn forget_pages(&mut self, file: &Resource) {
        let base = String::from(file.id.clone());
        let ours = |id: &ResourceId| String::from(id.clone()).starts_with(&format!("{base}#page"));
        self.kept.retain(|id, _| !ours(id));
        self.unkept.retain(|id| !ours(id));
        self.ready.retain(|id, _| !ours(id));
        self.keeping.retain(|id| !ours(id));
    }

    /// How many pages the paged file `id` has, once its pages are kept.
    pub fn pages(&self, id: &ResourceId) -> Option<usize> {
        match self.kept.get(id)? {
            Content::Pages(pages) => Some(pages.len()),
            _ => None,
        }
    }

    pub fn keep(
        &mut self,
        resources: impl IntoIterator<Item = Resource>,
    ) -> iced::Task<Arc<Loaded>> {
        let loaders: Vec<_> = resources
            .into_iter()
            .filter_map(|resource| match Loader::for_resource(&resource) {
                Ok(Some(loader)) if loader.is_still() => Some((resource.id, loader)),
                _ => None,
            })
            .collect();
        self.keeping = loaders.iter().map(|(id, _)| id.clone()).collect();
        self.kept.retain(|id, _| self.keeping.contains(id));
        self.unkept.retain(|id| self.keeping.contains(id));
        self.to_keep = loaders.into();
        self.keep_more()
    }

    /// Loads the next of the plan's pictures still to keep, a couple at a
    /// time. Called again as each load finishes.
    pub fn keep_more(&mut self) -> iced::Task<Arc<Loaded>> {
        let mut tasks = Vec::new();
        while self.keep_loading.len() < KEEP_AT_ONCE {
            let Some((id, loader)) = self.to_keep.pop_front() else {
                break;
            };
            // Kept or given up on already; or loading for the stage, which
            // keeps a copy when it lands.
            if self.kept.contains_key(&id)
                || self.unkept.contains(&id)
                || !self.loading.insert(id.clone())
            {
                continue;
            }
            self.keep_loading.insert(id.clone());
            tasks.push(spawn(id, loader, self.downloads.clone(), false, None));
        }
        iced::Task::batch(tasks)
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
        let mut tasks = Vec::new();
        for (id, loader) in loaders {
            // Not what's ready or kept loaded already, or loading.
            if self.ready.contains_key(&id)
                || self.kept.contains_key(&id)
                || !self.loading.insert(id.clone())
            {
                continue;
            }
            let stream = self.streams(&loader);
            if matches!(loader, Loader::Youtube(_)) {
                self.dwelling.insert(id.clone(), loader.clone());
            }
            let dwell = Some(self.wanted.clone());
            tasks.push(spawn(id, loader, self.downloads.clone(), stream, dwell));
        }
        self.sync_wanted();
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
pub(crate) fn open_video(uri: &url::Url) -> Result<Video, String> {
    open_playbin("playbin", uri, "", false)
}

/// The picture a live video is shown at, whatever quality arrives.
const LIVE_SIZE: (u32, u32) = (1280, 720);

/// The line speed a live video's player is told, in kbit/s: it picks the
/// best quality under it, which on YouTube is 720p, and stays there.
const LIVE_SPEED: u32 = 3000;

/// A player for a live video's manifest, with [`STREAM_START`] to show its
/// first picture.
///
/// Left to itself the player starts at the lowest quality and changes as
/// it measures the line, and a picture that changes size under the
/// renderer froze. So it is told a line speed, which fixes the quality,
/// and every picture is scaled to one size in case it changes anyway.
fn open_live(uri: &url::Url) -> Result<Video, String> {
    let (width, height) = LIVE_SIZE;
    open_playbin(
        &format!("playbin3 connection-speed={LIVE_SPEED}"),
        uri,
        &format!(",width={width},height={height}"),
        true,
    )
}

/// How wide a picture of `player` is against how tall, as it should be
/// seen: its pixels counted, and how much wider than tall each one is.
/// `None` until the player has a picture.
pub(crate) fn shape(player: &Video) -> Option<f32> {
    let (wide, tall) = player.size();
    if wide <= 0 || tall <= 0 {
        return None;
    }
    let pixel = player
        .pipeline()
        .property::<gst::Element>("video-sink")
        .downcast::<gst::Bin>()
        .ok()
        .and_then(|bin| bin.by_name("sink"))
        .and_then(|sink| sink.static_pad("sink"))
        .and_then(|pad| pad.current_caps())
        .and_then(|caps| {
            let shape = caps
                .structure(0)?
                .get::<gst::Fraction>("pixel-aspect-ratio")
                .ok()?;
            (shape.denom() > 0 && shape.numer() > 0)
                .then(|| shape.numer() as f32 / shape.denom() as f32)
        })
        .unwrap_or(1.0);
    Some(wide as f32 * pixel / tall as f32)
}

/// The largest size of `shape` (wide against tall) that fits `room`: all
/// of the room for a picture whose shape is not known yet.
fn fitted(room: iced::Size, shape: Option<f32>) -> (f32, f32) {
    match shape {
        Some(shape) if room.width > room.height * shape => (room.height * shape, room.height),
        Some(shape) => (room.width, room.width / shape),
        None => (room.width, room.height),
    }
}

/// `player`'s picture, as large as fits the room it is given and in the
/// middle of it, in the shape it should be seen: stretched there as it
/// is drawn, where its pixels are not square.
pub(crate) fn picture<'a, Message: Clone + 'a>(
    player: &'a Video,
    ended: Message,
    broke: impl Fn(String) -> Message + Clone + 'a,
) -> iced::Element<'a, Message> {
    let shape = shape(player);
    iced::widget::responsive(move |room| {
        let (wide, tall) = fitted(room, shape);
        let broke = broke.clone();
        iced::widget::container(
            iced_video_player::VideoPlayer::new(player)
                .width(wide)
                .height(tall)
                .content_fit(iced::ContentFit::Fill)
                .on_end_of_stream(ended.clone())
                .on_error(move |err| broke(err.message().to_string())),
        )
        .center(iced::Fill)
        .into()
    })
    .into()
}

/// [`open_video`] with the player element named, and `size` added to the
/// pictures it asks for. `patient`: it waits for the first picture.
fn open_playbin(playbin: &str, uri: &url::Url, size: &str, patient: bool) -> Result<Video, String> {
    let fail = |err: &dyn fmt::Display| format!("{uri}: {err}");
    gst::init().map_err(|err| fail(&err))?;
    // A player asked for a size has its pictures made that size, with
    // square pixels. One that plays them as they are leaves their pixels
    // the shape the file gives them: making them square would mean
    // working every picture over again, which for a large video costs
    // more than decoding it. They are stretched as they are drawn
    // instead, which costs nothing: see [`shape`].
    let pixels = match size {
        "" => "",
        _ => ",pixel-aspect-ratio=1/1",
    };
    let pipeline = gst::parse::launch(&format!(
        "{playbin} uri=\"{uri}\" mute=true video-sink=\"videoscale add-borders=true ! \
         videoconvert ! appsink name=sink drop=true \
         caps=video/x-raw,format=NV12{pixels}{size}\""
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
    if patient {
        wait_to_start(&pipeline).map_err(|err| fail(&err))?;
    }
    let mut video = Video::from_gst_pipeline(pipeline, sink, None).map_err(|err| fail(&err))?;
    video.set_paused(true);
    Ok(video)
}

/// How many pages the paged file `resource` has now, a PDF or a deck,
/// by rendering it (cached, by its size and when it changed). Blocks.
pub fn count_pages(resource: &Resource) -> Result<usize, String> {
    let pdf = match Loader::for_resource(resource)? {
        Some(Loader::Pdf(path, None)) => existing(&path)?.to_path_buf(),
        Some(Loader::Deck(path, None)) => {
            crate::deck::to_pdf(existing(&path)?, &cache_dir().join("decks"))?
        }
        _ => return Err("it is not a PDF or a presentation".to_string()),
    };
    render_pdf(&pdf, &cache_dir().join("pdf")).map(|pages| pages.len())
}

/// When the file `resource` is was last changed, and how big it is, in
/// a few words: two that differ are two versions of it. `None` for what
/// is not a file, or a file that can't be read.
pub fn stamp(resource: &Resource) -> Option<String> {
    let Location::File(path) = &resource.location else {
        return None;
    };
    let meta = std::fs::metadata(path).ok()?;
    let at = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some(format!(
        "{}.{:09}:{}",
        at.as_secs(),
        at.subsec_nanos(),
        meta.len()
    ))
}

/// The key a paged file's pages keep its [`stamp`] under, as it was when
/// they were opened out of it.
pub const STAMP: &str = "modified";

/// `pages`, or only the one `page` names (counted from 0): a page opened
/// out of its file into a step of its own.
fn only(pages: Vec<Handle>, page: Option<usize>) -> Result<Vec<Handle>, String> {
    match page {
        None => Ok(pages),
        Some(n) => pages
            .into_iter()
            .nth(n)
            .map(|one| vec![one])
            .ok_or_else(|| format!("its file has no page {} any more", n + 1)),
    }
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

    #[test]
    fn the_background_track_goes_down_and_back_up_over_a_moment() {
        let start = Instant::now();
        let down = Duck {
            from: 1.0,
            down: true,
            since: start,
        };
        assert_eq!(down.level(start), 1.0);
        let half = down.level(start + DUCK / 2);
        assert!((half - (1.0 + LOWERED) / 2.0).abs() < 0.01, "{half}");
        assert_eq!(down.level(start + DUCK), LOWERED);
        assert!(down.moving(start + DUCK / 2));
        assert!(!down.moving(start + DUCK));
        // Back up from wherever it got to.
        let up = Duck {
            from: half,
            down: false,
            since: start,
        };
        assert_eq!(up.level(start + DUCK * 2), 1.0);
        // Already where it is going, nothing moves.
        let still = Duck {
            from: 1.0,
            down: false,
            since: start,
        };
        assert!(!still.moving(start));
    }

    #[test]
    fn a_fade_runs_to_its_last_level_and_a_fade_out_moves_the_plan_on() {
        let text = |name: &str| {
            let source = SourceId::new("test").unwrap();
            Resource::new(
                ResourceId::new(&source, name),
                name,
                Kind::Text,
                Location::Inline(name.into()),
            )
        };
        let one_second = lantern_engine::FadeTime::from_secs(1);
        let stage_with = |fade: lantern_engine::Fade| {
            let engine = Engine::new();
            let next = engine.edit_plan(|plan| {
                let first = plan.show(text("Uno"));
                plan.get_mut(first).unwrap().fade = fade;
                plan.show(text("Dos"))
            });
            engine.next();
            let mut stage = Stage::new(Downloads::new(PathBuf::from("/nowhere")));
            let _load = stage.program_changed(&engine.stage(), &engine);
            (engine, stage, next)
        };

        let (up, mut rising, _) = stage_with(lantern_engine::Fade {
            fade_in: one_second,
            fade_out: None,
        });
        let state = up.stage();
        assert!(rising.fading(&state), "it comes up");
        rising.fade(&state, &up);
        assert!(rising.levels(&state).picture < 1.0);

        let (down, mut going, next) = stage_with(lantern_engine::Fade {
            fade_in: None,
            fade_out: one_second,
        });
        assert!(matches!(down.next(), lantern_engine::Moved::FadingOut(_)));
        going.sync_fades(&down.stage());
        assert!(!going.fade(&down.stage(), &down), "not faded yet");

        std::thread::sleep(Duration::from_millis(1100));
        assert!(
            rising.fading(&state),
            "up by now, but its last level is still to be played"
        );
        rising.fade(&state, &up);
        assert_eq!(rising.levels(&state).picture, 1.0);
        assert!(!rising.fading(&state));

        assert!(
            going.fade(&down.stage(), &down),
            "faded out, the plan went on"
        );
        assert_eq!(down.current_step().map(|step| step.id), Some(next));
    }

    #[test]
    fn a_picture_fills_the_room_it_fits_in_its_own_shape() {
        let wide = 2143.0 / 1290.0;
        // A room wider than the picture: as tall as the room.
        let (w, h) = fitted(iced::Size::new(960.0, 540.0), Some(wide));
        assert_eq!(h, 540.0);
        assert!((w / h - wide).abs() < 0.001 && w < 960.0);
        // A room taller than the picture: as wide as the room.
        let (w, h) = fitted(iced::Size::new(400.0, 540.0), Some(wide));
        assert_eq!(w, 400.0);
        assert!((w / h - wide).abs() < 0.001 && h < 540.0);
        // Not known yet: all of the room.
        assert_eq!(fitted(iced::Size::new(400.0, 300.0), None), (400.0, 300.0));
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/e2e/fixtures")
            .join(name)
    }

    fn resource(kind: Kind, location: Location) -> Resource {
        let source = SourceId::new("test").unwrap();
        Resource::new(ResourceId::new(&source, "x"), "X", kind, location)
    }

    /// A finished load of `resource`, as a worker would send it.
    fn finished(resource: &Resource, result: Result<Content, String>) -> Loaded {
        Loaded {
            resource: resource.id.clone(),
            result: Mutex::new(Some(result)),
        }
    }

    #[test]
    fn the_plans_pictures_stay_loaded_and_its_videos_dont() {
        let named = |name: &str, kind| {
            let source = SourceId::new("test").unwrap();
            let path = PathBuf::from(format!("/media/{name}"));
            Resource::new(
                ResourceId::new(&source, name),
                name,
                kind,
                Location::File(path),
            )
        };
        let picture = named("a.png", Kind::Image);
        let other = named("b.png", Kind::Image);
        let huge = named("c.png", Kind::Image);
        let video = named("d.mp4", Kind::Video);
        let pixels = |bytes: usize| Content::Image(Handle::from_rgba(1, 1, vec![0; bytes]));
        let engine = Engine::new();
        let state = StageState::default();
        let mut stage = Stage::new(Downloads::new(PathBuf::from("/nowhere")));
        let plan = [picture.clone(), other.clone(), huge.clone(), video.clone()];
        let _loads = stage.keep(plan.clone());
        assert_eq!(stage.loading.len(), KEEP_AT_ONCE, "a couple at a time");
        assert!(stage.keeping.contains(&picture.id));
        assert!(
            !stage.keeping.contains(&video.id),
            "a player each is too much"
        );

        let _again = stage.loaded(&finished(&picture, Ok(pixels(4))), &state, &engine);
        let _again = stage.loaded(
            &finished(&other, Err("file not found".into())),
            &state,
            &engine,
        );
        let _loads = stage.keep_more();
        assert!(stage.loading.contains(&huge.id), "then the next");
        let _again = stage.loaded(&finished(&huge, Ok(pixels(KEEP_BUDGET))), &state, &engine);
        assert!(stage.kept.contains_key(&picture.id));
        // What failed or didn't fit isn't loaded again on every change.
        assert!(stage.unkept.contains(&other.id) && stage.unkept.contains(&huge.id));
        let _loads = stage.keep(plan);
        assert!(stage.loading.is_empty(), "nothing left to try");
        // Shown, it's still kept: it can be shown again.
        assert!(
            stage
                .kept
                .get(&picture.id)
                .and_then(Content::still)
                .is_some()
        );

        // Out of the plan, it's let go; and tried afresh if it comes back.
        let _loads = stage.keep([other.clone()]);
        assert!(stage.kept.is_empty());
        assert_eq!(stage.unkept.len(), 1);
        let _loads = stage.keep([picture.clone()]);
        assert!(stage.unkept.is_empty());
        assert!(stage.loading.contains(&picture.id));
    }

    #[test]
    fn only_a_youtube_video_outside_the_plan_is_streamed() {
        let source = SourceId::new("link").unwrap();
        let loader = |id: &str| {
            let video = lantern_source_link::youtube_video(&source, id);
            Loader::for_resource(&video).unwrap().unwrap()
        };
        let mut stage = Stage::new(Downloads::new(PathBuf::from("/nowhere")));
        // Until it's said otherwise, everything is downloaded.
        assert!(!stage.streams(&loader("aaaaaaaaaaa")));
        stage.plan_videos(HashSet::from(["aaaaaaaaaaa".to_string()]), true);
        assert!(!stage.streams(&loader("aaaaaaaaaaa")), "the plan's own");
        assert!(stage.streams(&loader("bbbbbbbbbbb")), "shown on the fly");
        assert!(!stage.streams(&Loader::Video("/v.mp4".into())), "a file");
        stage.plan_videos(HashSet::new(), false);
        assert!(!stage.streams(&loader("bbbbbbbbbbb")), "switched off");
        assert_eq!(
            broke_reason(true, "Internal data stream error."),
            "the stream from YouTube stopped, is the network up? (Internal data stream error)"
        );
        assert_eq!(
            broke_reason(false, "Could not decode stream."),
            "the video stopped playing (Could not decode stream)"
        );
        let secs = Duration::from_secs;
        assert!(stopped_short(secs(50), secs(634)), "the network went");
        assert!(!stopped_short(secs(633), secs(634)), "its real end");
        assert!(!stopped_short(secs(8976), secs(8977)));
        assert!(!stopped_short(secs(3600), Duration::ZERO), "live");
        // No stream to play is an error, not a panic.
        assert!(open_stream(&Stream::Files(Vec::new())).is_err());
    }

    #[test]
    fn a_video_passed_over_isnt_loaded_and_one_taken_doesnt_wait() {
        let id = |name: &str| {
            let source = SourceId::new("link").unwrap();
            lantern_source_link::youtube_video(&source, name).id
        };
        let (passed, stayed, taken) = (id("aaaaaaaaaaa"), id("bbbbbbbbbbb"), id("ccccccccccc"));
        let wanted = Mutex::new(Wanted {
            upcoming: HashSet::from([stayed.clone()]),
            program: Some(taken.clone()),
        });
        let dwell = Duration::from_millis(60);
        let start = Instant::now();
        assert!(worth_loading(&wanted, &taken, Duration::from_secs(30)));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "no wait for the program"
        );
        let start = Instant::now();
        assert!(!worth_loading(&wanted, &passed, dwell));
        assert!(worth_loading(&wanted, &stayed, dwell));
        assert!(start.elapsed() >= dwell * 2, "each waited its dwell");
    }

    #[test]
    fn preloading_says_what_is_wanted_now() {
        let source = SourceId::new("link").unwrap();
        let video = |name: &str| lantern_source_link::youtube_video(&source, name);
        // The videos are "on disk", so nothing waits or asks YouTube.
        let dir = tempfile::tempdir().unwrap();
        for name in ["aaaaaaaaaaa", "bbbbbbbbbbb"] {
            std::fs::write(dir.path().join(format!("{name}.mp4")), b"").unwrap();
        }
        let mut stage = Stage::new(Downloads::new(dir.path().to_path_buf()));
        let (first, second) = (video("aaaaaaaaaaa"), video("bbbbbbbbbbb"));
        let _loads = stage.preload([first.clone()]);
        let _loads = stage.preload([second.clone()]);
        let wanted = stage.wanted.lock().unwrap();
        assert_eq!(wanted.upcoming, HashSet::from([second.id.clone()]));
        assert_eq!(wanted.program, None);
        assert!(stage.dwelling.contains_key(&first.id));
    }

    #[test]
    fn the_kind_and_location_pick_how_to_present() {
        let file = |kind, name: &str| resource(kind, Location::File(fixture(name)));
        assert_eq!(
            Loader::for_resource(&file(Kind::Video, "video.mp4")),
            Ok(Some(Loader::Video(fixture("video.mp4"))))
        );
        assert_eq!(
            Loader::for_resource(&file(Kind::Audio, "song.mp3")),
            Ok(Some(Loader::Audio(fixture("song.mp3"))))
        );
        assert_eq!(
            Loader::for_resource(&file(Kind::Document, "pages.pdf")),
            Ok(Some(Loader::Pdf(fixture("pages.pdf"), None)))
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
            .load(&downloads, false)
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
    #[test]
    fn a_page_opened_out_of_its_file_loads_alone() {
        let pages: Vec<Handle> = (0..3)
            .map(|n| Handle::from_path(format!("page-{n}.png")))
            .collect();
        assert_eq!(only(pages.clone(), None).unwrap().len(), 3);
        assert_eq!(only(pages.clone(), Some(1)).unwrap(), [pages[1].clone()]);
        assert!(
            only(pages, Some(5)).is_err(),
            "a page the file no longer has"
        );
        let deck = resource(Kind::Presentation, Location::File(fixture("deck.pptx"))).page(2);
        assert_eq!(
            Loader::for_resource(&deck),
            Ok(Some(Loader::Deck(fixture("deck.pptx"), Some(2))))
        );
    }
}
