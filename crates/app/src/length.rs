//! Lengths of time as the operator reads and types them.

use std::time::Duration;

/// Reads what the operator typed: `15` is fifteen minutes, `4:30` four and
/// a half. Empty means no length. `None` if it can't be read (yet: `4:`
/// while typing).
pub fn parse(text: &str) -> Option<Option<Duration>> {
    let text = text.trim();
    if text.is_empty() {
        return Some(None);
    }
    let (minutes, seconds) = match text.split_once(':') {
        Some((m, s)) if s.len() == 2 => (m.parse::<u64>().ok()?, s.parse::<u64>().ok()?),
        Some(_) => return None,
        None => (text.parse::<u64>().ok()?, 0),
    };
    (seconds < 60).then(|| Some(Duration::from_secs(minutes * 60 + seconds)))
}

/// `4:05`, `1:02:03` past an hour.
pub fn clock(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// What the operator typed for a length, back: `15` or `4:30`.
pub fn typed(d: Duration) -> String {
    let s = d.as_secs();
    if s.is_multiple_of(60) {
        (s / 60).to_string()
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// A guess, rounded to the minute: `~4 min`, `~1 min`, `~30 s` below one.
pub fn rough(d: Duration) -> String {
    match d.as_secs() {
        0..60 => format!("~{} s", d.as_secs()),
        s => format!("~{} min", (s + 30) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn minutes_and_seconds_read_as_typed() {
        assert_eq!(parse("15"), Some(Some(secs(900))));
        assert_eq!(parse(" 4:30 "), Some(Some(secs(270))));
        assert_eq!(parse(""), Some(None));
        assert_eq!(parse("4:"), None, "still typing");
        assert_eq!(parse("4:75"), None);
        assert_eq!(parse("abc"), None);
    }

    #[test]
    fn lengths_print_for_reading_and_for_editing() {
        assert_eq!(clock(secs(245)), "4:05");
        assert_eq!(clock(secs(3723)), "1:02:03");
        assert_eq!(typed(secs(900)), "15");
        assert_eq!(typed(secs(270)), "4:30");
        assert_eq!(rough(secs(250)), "~4 min");
        assert_eq!(rough(secs(30)), "~30 s");
        for text in ["15", "4:30"] {
            assert_eq!(typed(parse(text).unwrap().unwrap()), text, "round trip");
        }
    }
}
