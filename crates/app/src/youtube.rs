//! YouTube videos, downloaded ahead of time with `yt-dlp` into a cache
//! folder and played from there like a local file. A video needed before
//! its download is done waits for it. One shown outside the plan, or live,
//! plays straight from YouTube ([`stream`]).
//!
//! A video unused for [`KEEP`] is deleted when Lantern starts
//! ([`forget_unused`]). Using one (in the plan, previewed or played) marks
//! its file as used, so a video in a plan still in use stays.
//!
//! [`Downloads`] runs at most one download per video id and is shared by the
//! dashboard (which shows each video's [`Download`] state), the stage's
//! loader threads and the script.

use std::collections::HashMap;
use std::fmt;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use lantern_engine::{Location, Resource};

use crate::relay::Track;

/// H.264 up to 720p with AAC, which GStreamer plays without extra plugins.
const FORMAT: &str = "bv*[vcodec^=avc1][height<=720]+ba[ext=m4a]/b[ext=mp4]";

/// What a video shown outside the plan plays from, straight from YouTube:
/// one fixed H.264 video stream up to 720p and one AAC audio stream, both
/// plain HTTPS files, or failing that one file with both. Not the HLS
/// manifest: its player changes quality (and codec) as it goes, which
/// froze the picture and dropped the sound.
///
/// The sound is stereo when there is any. `ba` alone takes a surround
/// track when the video has one, and YouTube serves that so slowly at
/// first that the player gave up on it.
const STREAM_FORMAT: &str = concat!(
    "bv*[vcodec^=avc1][height<=720][protocol=https]",
    "+ba[ext=m4a][audio_channels<=2][protocol=https]",
    "/bv*[vcodec^=avc1][height<=720][protocol=https]+ba[ext=m4a][protocol=https]",
    "/b[ext=mp4][protocol=https]",
);

/// What a live video plays from when nothing above is on offer, which is
/// how a video that is live answers: it has no files, only manifests.
const LIVE_FORMAT: &str = "bv*[vcodec^=avc1][height<=720]+ba/b";

/// What a video plays from without being downloaded.
#[derive(Debug, Clone, PartialEq)]
pub enum Stream {
    /// The addresses of its video stream then its audio stream, or one
    /// address with both. They last a few hours.
    Files(Vec<String>),
    /// Its video stream and its audio stream with what a manifest for
    /// them needs, and its length in seconds. The player jumps in these
    /// at once, where in [`Files`](Stream::Files) it walks to the place.
    Pair {
        video: Track,
        audio: Track,
        seconds: f64,
    },
    /// A video that is live now: the address of its manifest, which lists
    /// the pieces of picture and sound as they are made.
    Live(String),
}

/// What to play `video` from without downloading it. Asks `yt-dlp`, which
/// takes a few seconds.
pub fn stream(video: &YoutubeVideo) -> Result<Stream, String> {
    let output = crate::programs::command("yt-dlp")
        .args(["-f", &format!("{STREAM_FORMAT}/{LIVE_FORMAT}")])
        .args(["--print", STREAM_FACTS, "--print", "urls"])
        .args(["--no-playlist", "--no-warnings"])
        .arg(&video.url)
        .output()
        .map_err(|err| format!("can't run yt-dlp: {err}"))?;
    if !output.status.success() {
        let errors = String::from_utf8_lossy(&output.stderr);
        return Err(failure(errors.lines(), output.status));
    }
    parse_stream(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| format!("yt-dlp gave no stream for {}", video.id))
}

/// The first line [`stream`] asks `yt-dlp` for, its parts between bars:
/// whether the video is live, its manifest's address if it has one, its
/// length in seconds, then the codec and bitrate (kbit/s) of its video
/// stream and of its audio stream. A part `yt-dlp` doesn't know is empty.
const STREAM_FACTS: &str = concat!(
    "%(is_live)s|%(requested_formats.0.manifest_url,manifest_url|)s|%(duration|)s",
    "|%(requested_formats.0.vcodec|)s|%(requested_formats.0.tbr|)s",
    "|%(requested_formats.1.acodec|)s|%(requested_formats.1.tbr|)s",
);

/// Reads what [`stream`] asked for: the [`STREAM_FACTS`] line, then the
/// addresses. A manifest is taken only from a video that is live: for any
/// other its player changes quality as it goes, and such a video is
/// better downloaded.
fn parse_stream(printed: &str) -> Option<Stream> {
    let mut lines = printed.lines().map(str::trim);
    let facts: Vec<&str> = lines.next()?.split('|').collect();
    let (live, manifest) = (*facts.first()?, *facts.get(1)?);
    if live == "True" {
        return manifest
            .starts_with("https://")
            .then(|| Stream::Live(manifest.to_string()));
    }
    let mut urls: Vec<String> = lines
        .filter(|line| line.starts_with("https://"))
        .map(str::to_string)
        .collect();
    if urls.iter().any(|url| url.contains("/manifest/")) || !matches!(urls.len(), 1 | 2) {
        return None;
    }
    // Two streams, and everything a manifest for them needs.
    let track = |url: String, codec: usize| {
        let bits = facts.get(codec + 1)?.parse::<f64>().ok()? * 1000.0;
        let codec = facts.get(codec).filter(|codec| !codec.is_empty())?;
        Some(Track {
            url,
            codec: codec.to_string(),
            bits: bits as u64,
        })
    };
    let seconds = facts.get(2).and_then(|seconds| seconds.parse::<f64>().ok());
    if let (2, Some(seconds)) = (urls.len(), seconds)
        && let Some(audio) = track(urls[1].clone(), 5)
        && let Some(video) = track(urls[0].clone(), 3)
    {
        urls.clear();
        return Some(Stream::Pair {
            video,
            audio,
            seconds,
        });
    }
    Some(Stream::Files(urls))
}

/// How long a downloaded video stays without being used.
pub const KEEP: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Deletes the files in `dir` last used more than `keep` before `now`,
/// half-finished downloads included, and returns what it deleted. A file's
/// modification time is its last use: [`Downloads::fetch`] sets it.
pub fn forget_unused(dir: &Path, keep: Duration, now: SystemTime) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            entry.metadata().is_ok_and(|meta| {
                meta.is_file()
                    && meta
                        .modified()
                        .is_ok_and(|used| now.duration_since(used).is_ok_and(|age| age > keep))
            })
        })
        .map(|entry| entry.path())
        .filter(|path| std::fs::remove_file(path).is_ok())
        .collect()
}

/// Marks `path` as used now.
fn touch(path: &Path) {
    let marked = std::fs::File::options()
        .append(true)
        .open(path)
        .and_then(|file| file.set_modified(SystemTime::now()));
    if let Err(err) = marked {
        crate::log(format!("youtube: {}: {err}", path.display()));
    }
}

/// A YouTube video, as the link source describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YoutubeVideo {
    /// The 11-character video id.
    pub id: String,
    url: String,
    /// Where the link said to start, in seconds (`t=` or `start=`).
    pub start: u32,
}

impl YoutubeVideo {
    /// The video behind `resource`, if it is a YouTube link.
    pub fn of(resource: &Resource) -> Option<YoutubeVideo> {
        let Location::Url(url) = &resource.location else {
            return None;
        };
        if resource.meta.get("provider").map(String::as_str) != Some("youtube") {
            return None;
        }
        let id = resource.meta.get("youtube_id")?.clone();
        let start = resource
            .meta
            .get("start_seconds")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        Some(YoutubeVideo {
            id,
            url: url.clone(),
            start,
        })
    }
}

/// Where one video's download stands.
#[derive(Debug, Clone, PartialEq)]
pub enum Download {
    /// Asked for; `yt-dlp` hasn't reported progress yet.
    Queued,
    /// Overall percent, across the video and audio streams.
    Running(f32),
    /// The finished file in the cache.
    Ready(PathBuf),
    /// It is live now, so there is nothing to download: it plays as it
    /// comes.
    Live,
    Failed(String),
}

impl fmt::Display for Download {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Download::Queued => f.write_str("queued"),
            Download::Running(percent) => write!(f, "downloading {percent:.0}%"),
            Download::Ready(_) => f.write_str("ready"),
            Download::Live => f.write_str("live"),
            Download::Failed(reason) => write!(f, "failed: {reason}"),
        }
    }
}

/// Every download this session asked for, by video id.
pub struct Downloads {
    dir: PathBuf,
    state: Mutex<HashMap<String, Download>>,
    changed: Condvar,
}

impl Downloads {
    /// Downloads into `dir`, which is created on the first download.
    pub fn new(dir: PathBuf) -> Arc<Downloads> {
        Arc::new(Downloads {
            dir,
            state: Mutex::new(HashMap::new()),
            changed: Condvar::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Download>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(&self, id: &str, download: Download) {
        self.lock().insert(id.to_string(), download);
        self.changed.notify_all();
    }

    /// The downloaded file of `video`, if it's on disk already: from this
    /// session or an earlier one. Marks it as used.
    pub fn cached(&self, video: &YoutubeVideo) -> Option<PathBuf> {
        if let Some(Download::Ready(path)) = self.state(&video.id) {
            return Some(path);
        }
        let cached = cache_path(&self.dir, &video.id);
        cached.is_file().then(|| {
            touch(&cached);
            cached
        })
    }

    pub fn state(&self, id: &str) -> Option<Download> {
        self.lock().get(id).cloned()
    }

    /// Whether any download is queued or running.
    pub fn busy(&self) -> bool {
        pending(&self.lock())
    }

    /// Starts downloading `video` on its own thread, unless it is cached,
    /// already on its way, or failed. A failed video starts again only after
    /// [`retry`](Downloads::retry).
    pub fn fetch(self: &Arc<Self>, video: &YoutubeVideo) {
        let mut state = self.lock();
        if state.contains_key(&video.id) {
            return;
        }
        let cached = cache_path(&self.dir, &video.id);
        if cached.is_file() {
            touch(&cached);
            state.insert(video.id.clone(), Download::Ready(cached));
            self.changed.notify_all();
            return;
        }
        state.insert(video.id.clone(), Download::Queued);
        drop(state);
        crate::log(format!("youtube: downloading {}", video.id));
        let downloads = Arc::clone(self);
        let video = video.clone();
        std::thread::spawn(move || {
            let result = download(&downloads.dir, &video, |percent| {
                downloads.set(&video.id, Download::Running(percent));
            });
            let download = match result {
                Ok(Some(path)) => Download::Ready(path),
                Ok(None) => Download::Live,
                Err(reason) => Download::Failed(reason),
            };
            crate::log(format!("youtube: {} {download}", video.id));
            downloads.set(&video.id, download);
        });
    }

    /// Forgets a failure, so the next [`fetch`](Downloads::fetch) tries again.
    pub fn retry(&self, video: &YoutubeVideo) {
        let mut state = self.lock();
        if matches!(state.get(&video.id), Some(Download::Failed(_))) {
            state.remove(&video.id);
        }
    }

    /// Fetches `video` and blocks until its file is ready or it failed.
    pub fn ready(self: &Arc<Self>, video: &YoutubeVideo) -> Result<PathBuf, String> {
        self.fetch(video);
        let mut state = self.lock();
        loop {
            match state.get(&video.id) {
                Some(Download::Ready(path)) => return Ok(path.clone()),
                Some(Download::Failed(reason)) => return Err(reason.clone()),
                Some(Download::Live) => return Err("it is live".into()),
                None => return Err("the download was cancelled".into()),
                Some(Download::Queued | Download::Running(_)) => {}
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Blocks until no download is queued or running, for at most `timeout`.
    /// Returns whether that happened in time.
    pub fn settle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if !pending(&state) {
                return true;
            }
            if left.is_zero() {
                return false;
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

fn pending(state: &HashMap<String, Download>) -> bool {
    state
        .values()
        .any(|d| matches!(d, Download::Queued | Download::Running(_)))
}

/// The finished file for a video id.
fn cache_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.mp4"))
}

/// Where `yt-dlp` writes until it is done. Renamed to [`cache_path`] after,
/// so a half-written file is never played.
fn partial_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.download.mp4"))
}

/// What `yt-dlp` said went wrong: its last `ERROR:` line, or how it exited.
fn failure<'a>(stderr: impl IntoIterator<Item = &'a str>, status: ExitStatus) -> String {
    let reason = stderr
        .into_iter()
        .filter_map(|line| line.strip_prefix("ERROR: "))
        .last()
        .map_or_else(|| status.to_string(), str::to_string);
    format!("yt-dlp: {reason}")
}

/// Runs `yt-dlp`, calling `progress` with the overall percent. `None` for
/// a video that is live, which `yt-dlp` is told to leave alone: its
/// download would go on for as long as it is live.
fn download(
    dir: &Path,
    video: &YoutubeVideo,
    mut progress: impl FnMut(f32),
) -> Result<Option<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let partial = partial_path(dir, &video.id);
    let mut child = crate::programs::command("yt-dlp")
        .args(["-f", FORMAT, "--merge-output-format", "mp4"])
        .args(["--newline", "--progress", "--no-playlist"])
        .args(["--match-filter", "!is_live"])
        .arg("-o")
        .arg(dir.join(format!("{}.download.%(ext)s", video.id)))
        .arg(&video.url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("can't run yt-dlp: {err}"))?;
    let stderr = child.stderr.take();
    let errors = std::thread::spawn(move || {
        let lines = stderr.into_iter().flat_map(|s| BufReader::new(s).lines());
        lines.map_while(Result::ok).collect::<Vec<String>>()
    });
    let mut tracker = Progress::default();
    let mut live = false;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            live |= line.contains("does not pass filter (!is_live)");
            if let Some(percent) = tracker.line(&line) {
                progress(percent);
            }
        }
    }
    let status = child.wait().map_err(|err| format!("yt-dlp: {err}"))?;
    let errors = errors.join().unwrap_or_default();
    if !status.success() {
        return Err(failure(errors.iter().map(String::as_str), status));
    }
    if live {
        return Ok(None);
    }
    let done = cache_path(dir, &video.id);
    std::fs::rename(&partial, &done).map_err(|err| format!("{}: {err}", partial.display()))?;
    Ok(Some(done))
}

/// Follows `yt-dlp --newline` output across its streams: a video and an
/// audio stream download one after the other, each from 0 to 100%.
#[derive(Debug, Default)]
struct Progress {
    /// From `[info] ID: Downloading 1 format(s): 136+140`.
    streams: usize,
    /// Streams started so far, counted by `[download] Destination:` lines.
    started: usize,
}

impl Progress {
    /// The overall percent, if `line` moves it.
    fn line(&mut self, line: &str) -> Option<f32> {
        if let Some(rest) = line.strip_prefix("[info] ") {
            let formats = rest.rsplit_once("format(s): ")?.1;
            self.streams = formats.split('+').count();
            return None;
        }
        let rest = line.strip_prefix("[download] ")?.trim_start();
        if rest.starts_with("Destination:") {
            self.started += 1;
            return None;
        }
        let percent: f32 = rest.split_once('%')?.0.parse().ok()?;
        let streams = self.streams.max(self.started).max(1);
        let done = self.started.saturating_sub(1).min(streams - 1);
        Some((done as f32 * 100.0 + percent.clamp(0.0, 100.0)) / streams as f32)
    }
}

#[cfg(test)]
mod tests {
    use lantern_engine::{Kind, ResourceId, SourceId};

    use super::*;

    #[test]
    fn what_yt_dlp_prints_says_how_a_video_streams() {
        let pair = "False||634.5|avc1.4D401F|1101.2|mp4a.40.2|129.5\n\
                    https://a.googlevideo.com/v\nhttps://a.googlevideo.com/a\n";
        assert_eq!(
            parse_stream(pair),
            Some(Stream::Pair {
                video: Track {
                    url: "https://a.googlevideo.com/v".into(),
                    codec: "avc1.4D401F".into(),
                    bits: 1_101_200,
                },
                audio: Track {
                    url: "https://a.googlevideo.com/a".into(),
                    codec: "mp4a.40.2".into(),
                    bits: 129_500,
                },
                seconds: 634.5,
            })
        );
        // Without all a manifest needs, the two files as they are.
        let files = "False||634.5|avc1.4D401F||mp4a.40.2|129.5\n\
                     https://a.googlevideo.com/v\nhttps://a.googlevideo.com/a\n";
        assert_eq!(
            parse_stream(files),
            Some(Stream::Files(vec![
                "https://a.googlevideo.com/v".into(),
                "https://a.googlevideo.com/a".into()
            ]))
        );
        assert_eq!(
            parse_stream("NA||19|||||\nhttps://a.googlevideo.com/both\n"),
            Some(Stream::Files(vec!["https://a.googlevideo.com/both".into()]))
        );
        // Live: its manifest, not the two lists of pieces under it.
        let live = "True|https://manifest.googlevideo.com/api/manifest/hls_variant/x.m3u8|||||\n\
                    https://manifest.googlevideo.com/api/manifest/hls_playlist/v.m3u8\n\
                    https://manifest.googlevideo.com/api/manifest/hls_playlist/a.m3u8\n";
        assert_eq!(
            parse_stream(live),
            Some(Stream::Live(
                "https://manifest.googlevideo.com/api/manifest/hls_variant/x.m3u8".into()
            ))
        );
        // A manifest of a video that isn't live is no stream to play.
        assert_eq!(parse_stream(&live.replacen("True", "False", 1)), None);
        assert_eq!(parse_stream("True|\n"), None);
        assert_eq!(parse_stream("False|\n"), None);
        assert_eq!(parse_stream(""), None);
    }

    #[test]
    fn a_stream_takes_stereo_sound_before_any_other() {
        let choices: Vec<&str> = STREAM_FORMAT.split('/').collect();
        assert!(choices[0].contains("[audio_channels<=2]"), "{choices:?}");
        assert!(!choices[1].contains("audio_channels"), "{choices:?}");
        // Never the manifest, whose player changes quality as it goes.
        assert!(choices.iter().all(|c| c.ends_with("[protocol=https]")));
    }

    #[test]
    fn progress_spans_the_video_and_audio_streams() {
        let mut progress = Progress::default();
        let mut feed = |line: &str| progress.line(line);
        assert_eq!(feed("[info] abc: Downloading 1 format(s): 136+140"), None);
        assert_eq!(feed("[download] Destination: x.f136.mp4"), None);
        assert_eq!(
            feed("[download]  50.0% of 1.00MiB at 1MiB/s ETA 00:01"),
            Some(25.0)
        );
        assert_eq!(feed("[download] 100% of 1.00MiB in 00:00:01"), Some(50.0));
        assert_eq!(feed("[download] Destination: x.f140.m4a"), None);
        assert_eq!(
            feed("[download]  50.0% of 1.00MiB at 1MiB/s ETA 00:01"),
            Some(75.0)
        );
        assert_eq!(feed("[Merger] Merging formats into \"x.mp4\""), None);
    }

    #[test]
    fn a_single_stream_counts_on_its_own() {
        let mut progress = Progress::default();
        progress.line("[info] abc: Downloading 1 format(s): 18");
        progress.line("[download] Destination: x.mp4");
        assert_eq!(
            progress.line("[download]  41.5% of  614.43KiB at 336.55KiB/s ETA 00:01"),
            Some(41.5)
        );
        assert_eq!(progress.line("[youtube] abc: Downloading webpage"), None);
    }

    #[test]
    fn the_cache_is_named_by_video_id() {
        let dir = Path::new("/cache/youtube");
        assert_eq!(
            cache_path(dir, "jNQXAC9IVRw"),
            Path::new("/cache/youtube/jNQXAC9IVRw.mp4")
        );
        assert_ne!(
            partial_path(dir, "jNQXAC9IVRw"),
            cache_path(dir, "jNQXAC9IVRw")
        );
    }

    fn link(url: &str) -> Resource {
        let source = SourceId::new("link").unwrap();
        Resource::new(
            ResourceId::new(&source, url),
            "x",
            Kind::Video,
            Location::Url(url.into()),
        )
    }

    #[test]
    fn only_youtube_links_are_youtube_videos() {
        let url = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
        let youtube = link(url)
            .with_meta("provider", "youtube")
            .with_meta("youtube_id", "jNQXAC9IVRw");
        let video = YoutubeVideo::of(&youtube).unwrap();
        assert_eq!(video.id, "jNQXAC9IVRw");
        assert_eq!(video.url, url);
        assert_eq!(YoutubeVideo::of(&link("https://example.com/a.mp4")), None);
    }

    #[test]
    fn videos_unused_for_a_long_time_are_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let day = Duration::from_secs(24 * 60 * 60);
        let aged = |name: &str, days: u32| {
            let path = dir.path().join(name);
            std::fs::write(&path, "").unwrap();
            let file = std::fs::File::options().append(true).open(&path).unwrap();
            file.set_modified(now - day * days).unwrap();
            path
        };
        let old = aged("old.mp4", 40);
        let recent = aged("recent.mp4", 3);
        let stuck = aged("half.download.mp4", 31);
        let mut gone = forget_unused(dir.path(), KEEP, now);
        gone.sort();
        assert_eq!(gone, [stuck.clone(), old.clone()]);
        assert!(recent.is_file() && !old.exists() && !stuck.exists());
        assert!(forget_unused(&dir.path().join("missing"), KEEP, now).is_empty());
    }

    #[test]
    fn using_a_cached_video_marks_it_as_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_path(dir.path(), "jNQXAC9IVRw");
        std::fs::write(&path, "").unwrap();
        let long_ago = SystemTime::now() - KEEP * 2;
        std::fs::File::options()
            .append(true)
            .open(&path)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();
        let video = YoutubeVideo {
            id: "jNQXAC9IVRw".into(),
            url: "unused".into(),
            start: 0,
        };
        Downloads::new(dir.path().to_path_buf()).fetch(&video);
        assert!(forget_unused(dir.path(), KEEP, SystemTime::now()).is_empty());
        assert!(path.is_file());
    }

    #[test]
    fn a_cached_file_is_ready_without_downloading() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(cache_path(dir.path(), "jNQXAC9IVRw"), "").unwrap();
        let downloads = Downloads::new(dir.path().to_path_buf());
        let video = YoutubeVideo {
            id: "jNQXAC9IVRw".into(),
            url: "unused".into(),
            start: 0,
        };
        assert_eq!(
            downloads.ready(&video),
            Ok(cache_path(dir.path(), "jNQXAC9IVRw"))
        );
        assert!(!downloads.busy());
        assert!(downloads.settle(Duration::ZERO));
    }

    #[test]
    fn a_failure_is_the_last_error_line() {
        #[cfg(unix)]
        let status = {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(1 << 8)
        };
        #[cfg(windows)]
        let status = {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(1)
        };
        let stderr = "WARNING: slow\nERROR: first\nERROR: [youtube] x: Video unavailable\n";
        assert_eq!(
            failure(stderr.lines(), status),
            "yt-dlp: [youtube] x: Video unavailable"
        );
        assert_eq!(failure(["no errors"], status), format!("yt-dlp: {status}"));
    }
}
