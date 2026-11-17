//! The look round as a service starts: what would go wrong in front of
//! everyone, said while there is still time. The plan's steps that won't
//! show, and the sound: muted, at nothing, or going somewhere it didn't
//! go last time.
//!
//! It warns and never refuses. With nothing to say, the service starts.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ready::Level;

/// Where sound goes now, as PipeWire says it.
#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    /// The output's name as the desktop shows it ("Built-in Speakers").
    pub output: String,
    /// 1 is full.
    pub volume: f32,
    pub muted: bool,
}

/// The default sound output, asked of `wpctl`. `None` where there is no
/// PipeWire to ask: then nothing is said about sound.
pub fn sound() -> Option<Sound> {
    // A test that starts a service must not depend on whether the
    // machine it runs on is muted.
    if cfg!(test) {
        return None;
    }
    let ask = |args: &[&str]| {
        let out = Command::new("wpctl").args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let (volume, muted) = volume(&ask(&["get-volume", "@DEFAULT_AUDIO_SINK@"])?)?;
    let output = ask(&["inspect", "@DEFAULT_AUDIO_SINK@"])
        .and_then(|text| described(&text))
        .unwrap_or_default();
    Some(Sound {
        output,
        volume,
        muted,
    })
}

/// `Volume: 0.56 [MUTED]` as its level and whether it is muted.
fn volume(line: &str) -> Option<(f32, bool)> {
    let mut words = line.split_whitespace().skip(1);
    let level = words.next()?.parse().ok()?;
    Some((level, words.any(|word| word == "[MUTED]")))
}

/// The `node.description` in what `wpctl inspect` prints.
fn described(inspect: &str) -> Option<String> {
    let (_, value) = inspect
        .lines()
        .find(|line| line.contains("node.description"))?
        .split_once('=')?;
    let name = value.trim().trim_matches('"');
    (!name.is_empty()).then(|| name.to_string())
}

/// Where sound went as the last service started, kept between runs.
pub struct LastOutput {
    file: PathBuf,
}

impl LastOutput {
    pub fn new(data: &Path) -> LastOutput {
        LastOutput {
            file: data.join("sound-output"),
        }
    }

    pub fn get(&self) -> Option<String> {
        let kept = std::fs::read_to_string(&self.file).ok()?;
        let kept = kept.trim();
        (!kept.is_empty()).then(|| kept.to_string())
    }

    pub fn keep(&self, output: &str) -> std::io::Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.file, format!("{output}\n"))
    }
}

/// A step that isn't ready: how bad, its name, and why.
pub struct Trouble {
    pub level: Level,
    pub label: String,
    pub what: String,
}

/// What to say before the service starts, worst first: empty when it can
/// just start. `sound` is `None` when the plan
/// plays none or there is no way to ask; `last` is where sound went last
/// service.
pub fn worries(issues: &[Trouble], sound: Option<&Sound>, last: Option<&str>) -> Vec<String> {
    let mut worries = Vec::new();
    for (level, one, many) in [
        (Level::Problem, "won't show", "steps won't show"),
        (
            Level::Waiting,
            "isn't downloaded yet",
            "steps aren't downloaded yet",
        ),
    ] {
        let of: Vec<&Trouble> = issues.iter().filter(|issue| issue.level == level).collect();
        match of.as_slice() {
            [] => {}
            [issue] => worries.push(format!("“{}” {one}: {}", issue.label, issue.what)),
            more => worries.push(format!("{} {many}", more.len())),
        }
    }
    if let Some(sound) = sound {
        if sound.muted {
            worries.push("The sound is muted".into());
        } else if sound.volume < 0.01 {
            worries.push("The volume is at 0".into());
        }
        match last {
            Some(last) if !sound.output.is_empty() && last != sound.output => worries.push(
                format!("Sound goes to {}. Last service: {last}", sound.output),
            ),
            _ => {}
        }
    }
    worries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(level: Level, what: &str) -> Trouble {
        Trouble {
            level,
            label: "Himno 99".into(),
            what: what.into(),
        }
    }

    fn sound(output: &str, volume: f32, muted: bool) -> Sound {
        Sound {
            output: output.into(),
            volume,
            muted,
        }
    }

    #[test]
    fn reads_what_wpctl_prints() {
        assert_eq!(volume("Volume: 0.56\n"), Some((0.56, false)));
        assert_eq!(volume("Volume: 0.40 [MUTED]\n"), Some((0.40, true)));
        assert_eq!(volume("nonsense"), None);
        let inspect = "id 55, type PipeWire:Interface:Node\n  * node.description = \"HD Audio Speaker\"\n  * node.name = \"alsa_output.x\"\n";
        assert_eq!(described(inspect).as_deref(), Some("HD Audio Speaker"));
    }

    #[test]
    fn nothing_to_say_starts_the_service() {
        let fine = sound("Speakers", 0.5, false);
        assert!(worries(&[], Some(&fine), Some("Speakers")).is_empty());
        // The first service ever has no last output to differ from.
        assert!(worries(&[], Some(&fine), None).is_empty());
        // No way to ask about sound: nothing said about it.
        assert!(worries(&[], None, Some("Speakers")).is_empty());
    }

    #[test]
    fn says_what_is_wrong_worst_first() {
        let issues = [
            issue(Level::Waiting, "not downloaded yet"),
            issue(Level::Problem, "file missing"),
        ];
        let said = worries(&issues, Some(&sound("HDMI", 0.5, true)), Some("Speakers"));
        assert_eq!(
            said,
            [
                "“Himno 99” won't show: file missing",
                "“Himno 99” isn't downloaded yet: not downloaded yet",
                "The sound is muted",
                "Sound goes to HDMI. Last service: Speakers",
            ]
        );
    }

    #[test]
    fn counts_when_there_are_several() {
        let issues = [
            issue(Level::Problem, "file missing"),
            issue(Level::Problem, "file missing"),
        ];
        let said = worries(&issues, Some(&sound("", 0.0, false)), None);
        assert_eq!(said, ["2 steps won't show", "The volume is at 0"]);
    }

    #[test]
    fn keeps_the_last_output() {
        let dir = tempfile::tempdir().unwrap();
        let last = LastOutput::new(&dir.path().join("lantern"));
        assert_eq!(last.get(), None);
        last.keep("HDMI").unwrap();
        assert_eq!(last.get().as_deref(), Some("HDMI"));
    }
}
