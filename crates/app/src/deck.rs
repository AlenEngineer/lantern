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
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
    let meta = std::fs::metadata(path).map_err(|err| fail(&err))?;
    let mut key = DefaultHasher::new();
    (path, meta.len(), meta.modified().ok()).hash(&mut key);
    let dir = cache.join(format!("{:016x}", key.finish()));
    let pdf = dir.join("deck.pdf");
    let _one = CONVERTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if pdf.is_file() {
        return Ok(pdf);
    }
    let _ = std::fs::remove_dir_all(&dir);
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
