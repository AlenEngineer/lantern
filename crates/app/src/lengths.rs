//! How long each video and sound file in the library plays, measured once
//! and kept.
//!
//! A file's length is only known once something opens it, and filling a
//! section to its time needs the length of every file it could pick. So
//! each is opened once, off to the side and silently, and what it says is
//! kept in `lengths` in the data folder, by path and by when the file was
//! last changed. One is measured at a time: this must never get in the
//! way of what is on the projector.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use gstreamer as gst;
use gstreamer::prelude::*;

/// How long a file may take to say its length before it is given up on.
const PATIENCE: gst::ClockTime = gst::ClockTime::from_seconds(10);

/// A file's length, good for as long as the file isn't changed: seconds
/// since the epoch of its last change, and its length. A file that can't
/// say is kept too, as `None`, so it isn't asked again.
type Known = HashMap<PathBuf, (u64, Option<Duration>)>;

/// The lengths known so far, and the thread that measures the rest.
pub struct Lengths {
    known: Arc<Mutex<Known>>,
    /// Files waiting to be measured, or being measured.
    asked: Mutex<Vec<PathBuf>>,
    to: Sender<PathBuf>,
}

/// When `path` was last changed, in seconds since the epoch.
fn changed(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_secs(),
    )
}

/// One kept length as a line: path, when changed, milliseconds or `-`.
fn line(path: &Path, changed: u64, length: Option<Duration>) -> String {
    let length = length.map_or("-".to_string(), |length| length.as_millis().to_string());
    format!("{}\t{changed}\t{length}\n", path.display())
}

/// Reads the kept lengths. A line that doesn't read is passed over.
fn read(text: &str) -> Known {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.rsplitn(3, '\t');
            let length = parts.next()?;
            let changed = parts.next()?.parse().ok()?;
            let path = PathBuf::from(parts.next()?);
            let length = match length {
                "-" => None,
                millis => Some(Duration::from_millis(millis.parse().ok()?)),
            };
            Some((path, (changed, length)))
        })
        .collect()
}

impl Lengths {
    /// Reads what is kept in `data` and starts the thread that measures.
    pub fn start(data: &Path) -> Lengths {
        let file = data.join("lengths");
        let known = Arc::new(Mutex::new(read(
            &std::fs::read_to_string(&file).unwrap_or_default(),
        )));
        let (to, from) = mpsc::channel::<PathBuf>();
        let keep = known.clone();
        std::thread::spawn(move || {
            for path in from {
                let Some(changed) = changed(&path) else {
                    continue;
                };
                let length = measure(&path);
                keep.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(path.clone(), (changed, length));
                // Added to the end: the last line for a path is the one.
                if let Some(dir) = file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let kept = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&file)
                    .and_then(|mut file| {
                        std::io::Write::write_all(
                            &mut file,
                            line(&path, changed, length).as_bytes(),
                        )
                    });
                if let Err(error) = kept {
                    crate::log(format!("lengths: not kept: {error}"));
                }
            }
        });
        Lengths {
            known,
            asked: Mutex::new(Vec::new()),
            to,
        }
    }

    /// How long `path` plays, if that is known for the file as it is now.
    /// Unknown, it is asked for: a later call may have it.
    pub fn of(&self, path: &Path) -> Option<Duration> {
        let now = changed(path)?;
        let known = self
            .known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(path)
            .copied();
        match known {
            Some((then, length)) if then == now => length,
            _ => {
                let mut asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
                if !asked.iter().any(|asked| asked == path) {
                    asked.push(path.to_path_buf());
                    let _ = self.to.send(path.to_path_buf());
                }
                None
            }
        }
    }
}

/// Opens `path` far enough to say how long it plays, with its picture
/// and its sound going nowhere.
fn measure(path: &Path) -> Option<Duration> {
    gst::init().ok()?;
    let uri = url::Url::from_file_path(path).ok()?;
    let nowhere = || gst::ElementFactory::make("fakesink").build().ok();
    let player = gst::ElementFactory::make("playbin")
        .property("uri", uri.as_str())
        .property("mute", true)
        .property("video-sink", nowhere()?)
        .property("audio-sink", nowhere()?)
        .build()
        .ok()?;
    let length = player
        .set_state(gst::State::Paused)
        .ok()
        .filter(|_| player.state(PATIENCE).0.is_ok())
        .and_then(|_| player.query_duration::<gst::ClockTime>())
        .map(|length| Duration::from_nanos(length.nseconds()))
        .filter(|length| !length.is_zero());
    let _ = player.set_state(gst::State::Null);
    length
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_lengths_read_back_and_the_last_line_for_a_file_is_the_one() {
        let hymn = Path::new("/music/Himno 99\t(tab).mp4");
        let text = [
            line(hymn, 100, Some(Duration::from_millis(187_250))),
            line(Path::new("/music/broken.mp4"), 7, None),
            "not a line\n".to_string(),
            line(hymn, 200, Some(Duration::from_secs(190))),
        ]
        .concat();
        let known = read(&text);
        assert_eq!(known.len(), 2);
        assert_eq!(known[hymn], (200, Some(Duration::from_secs(190))));
        assert_eq!(known[Path::new("/music/broken.mp4")], (7, None));
    }

    /// Run by hand against a real file:
    /// `LANTERN_TEST_MEDIA=/path/to/file cargo test -p lantern-app measures -- --ignored`
    #[test]
    #[ignore = "needs a media file named by LANTERN_TEST_MEDIA"]
    fn measures_a_real_file_and_keeps_it() {
        let media = PathBuf::from(std::env::var("LANTERN_TEST_MEDIA").unwrap());
        let length = measure(&media).expect("it says how long it plays");
        assert!(length > Duration::from_secs(1), "{length:?}");
        // Asked through the cache: unknown at first, then kept.
        let dir = tempfile::tempdir().unwrap();
        let lengths = Lengths::start(dir.path());
        assert_eq!(lengths.of(&media), None);
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while lengths.of(&media).is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(lengths.of(&media), Some(length));
        let kept = std::fs::read_to_string(dir.path().join("lengths")).unwrap();
        assert_eq!(read(&kept)[&media].1, Some(length));
        println!("{} plays {length:?}", media.display());
    }
}
