//! YouTube videos, downloaded ahead of time with `yt-dlp` into a cache
//! folder and played from there like a local file. A video needed before
//! its download is done waits for it.
//!
//! [`Downloads`] runs at most one download per video id and is shared by the
//! dashboard (which shows each video's [`Download`] state), the stage's
//! loader threads and the script.

use std::collections::HashMap;
use std::fmt;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use lantern_engine::{Location, Resource};

/// H.264 up to 720p with AAC, which GStreamer plays without extra plugins.
const FORMAT: &str = "bv*[vcodec^=avc1][height<=720]+ba[ext=m4a]/b[ext=mp4]";

/// A YouTube video, as the link source describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YoutubeVideo {
    /// The 11-character video id.
    pub id: String,
    url: String,
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
        Some(YoutubeVideo {
            id,
            url: url.clone(),
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
    Failed(String),
}

impl fmt::Display for Download {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Download::Queued => f.write_str("queued"),
            Download::Running(percent) => write!(f, "downloading {percent:.0}%"),
            Download::Ready(_) => f.write_str("ready"),
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
                Ok(path) => Download::Ready(path),
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

/// Runs `yt-dlp`, calling `progress` with the overall percent.
fn download(
    dir: &Path,
    video: &YoutubeVideo,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let partial = partial_path(dir, &video.id);
    let mut child = Command::new("yt-dlp")
        .args(["-f", FORMAT, "--merge-output-format", "mp4"])
        .args(["--newline", "--progress", "--no-playlist"])
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
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
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
    let done = cache_path(dir, &video.id);
    std::fs::rename(&partial, &done).map_err(|err| format!("{}: {err}", partial.display()))?;
    Ok(done)
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
    fn a_cached_file_is_ready_without_downloading() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(cache_path(dir.path(), "jNQXAC9IVRw"), "").unwrap();
        let downloads = Downloads::new(dir.path().to_path_buf());
        let video = YoutubeVideo {
            id: "jNQXAC9IVRw".into(),
            url: "unused".into(),
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
        use std::os::unix::process::ExitStatusExt;
        let status = ExitStatus::from_raw(1 << 8);
        let stderr = "WARNING: slow\nERROR: first\nERROR: [youtube] x: Video unavailable\n";
        assert_eq!(
            failure(stderr.lines(), status),
            "yt-dlp: [youtube] x: Video unavailable"
        );
        assert_eq!(failure(["no errors"], status), "yt-dlp: exit status: 1");
    }
}
