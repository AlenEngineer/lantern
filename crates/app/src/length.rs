//! Lengths of time as the operator reads and types them.

use std::time::Duration;

/// Reads what the operator typed: `15` is fifteen minutes, `4:30` four and
/// a half. With hours: `3h` is three hours, `3h15` (or `3h 15`, `3h15m`)
/// three and a quarter, `1:02:03` an hour, two minutes and three seconds;
/// `45m` is forty-five minutes. Empty means no length. `None` if it can't
/// be read (yet: `4:` while typing).
pub fn parse(text: &str) -> Option<Option<Duration>> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return Some(None);
    }
    // In words: hours, then minutes.
    if text.contains('h') || text.ends_with('m') {
        let (hours, rest) = match text.split_once('h') {
            Some((hours, rest)) => (hours.trim().parse::<u64>().ok()?, rest.trim()),
            None => (0, text.as_str()),
        };
        let rest = rest.strip_suffix('m').unwrap_or(rest).trim();
        let minutes = match rest {
            "" => 0,
            minutes => minutes.parse::<u64>().ok()?,
        };
        return Some(Some(Duration::from_secs(hours * 3600 + minutes * 60)));
    }
    // As a clock: minutes and seconds, or hours, minutes and seconds.
    let parts: Vec<&str> = text.split(':').collect();
    let two = |part: &str| {
        (part.len() == 2)
            .then(|| part.parse::<u64>().ok())
            .flatten()
    };
    let (hours, minutes, seconds) = match parts[..] {
        [minutes] => (0, minutes.parse::<u64>().ok()?, 0),
        [minutes, seconds] => (0, minutes.parse::<u64>().ok()?, two(seconds)?),
        [hours, minutes, seconds] => {
            let minutes = two(minutes).filter(|minutes| *minutes < 60)?;
            (hours.parse::<u64>().ok()?, minutes, two(seconds)?)
        }
        _ => return None,
    };
    (seconds < 60).then(|| Some(Duration::from_secs(hours * 3600 + minutes * 60 + seconds)))
}

/// Whether `text` could still become a length: digits, up to two colons,
/// or an `h` and an `m` with spaces. A length field takes nothing else,
/// so it never holds what [`parse`] can't read once finished.
pub fn typeable(text: &str) -> bool {
    let text = text.to_lowercase();
    let only = |c: char| text.matches(c).count();
    text.chars()
        .all(|c| c.is_ascii_digit() || matches!(c, ':' | 'h' | 'm' | ' '))
        && only(':') <= 2
        && only('h') <= 1
        && only('m') <= 1
        // One way of saying it or the other.
        && (only(':') == 0 || only('h') + only('m') == 0)
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

/// The time of day now, here, as time since midnight.
pub fn now_of_day() -> Duration {
    use chrono::Timelike;
    Duration::from_secs(u64::from(
        chrono::Local::now().time().num_seconds_from_midnight(),
    ))
}

/// A time of day as a clock shows it: `10:52`.
pub fn clock_of_day(since_midnight: Duration) -> String {
    let minutes = since_midnight.as_secs() / 60;
    format!("{}:{:02}", minutes / 60 % 24, minutes % 60)
}

/// How far a service is from its schedule, in words: "on time" within a
/// minute, "4 min behind", "2 min ahead".
pub fn drift(behind: i64) -> String {
    let by = rough(Duration::from_secs(behind.unsigned_abs()));
    let by = by.trim_start_matches('~');
    match behind {
        -59..=59 => "on time".into(),
        60.. => format!("{by} behind"),
        _ => format!("{by} ahead"),
    }
}

/// What to say of a start time while the service hasn't begun: "starts
/// 10:30 · in 12 min" from three hours before, "starts 10:30 · 5 min
/// ago" for an hour after, and nothing the rest of the day. `until` is the
/// seconds to go, negative once past.
pub fn starting(starts: &str, until: i64) -> Option<String> {
    let span = rough(Duration::from_secs(until.unsigned_abs()));
    let span = span.trim_start_matches('~');
    match until {
        -3600..=-60 => Some(format!("starts {starts} · {span} ago")),
        -59..=59 => Some(format!("starts {starts} · now")),
        60..=10800 => Some(format!("starts {starts} · in {span}")),
        _ => None,
    }
}

/// What the operator typed for a length, back: `15` or `4:30`.
pub fn typed(d: Duration) -> String {
    let s = d.as_secs();
    let (hours, minutes) = (s / 3600, s / 60 % 60);
    match (s.is_multiple_of(60), hours) {
        // Whole minutes under an hour: the number alone.
        (true, 0) => (s / 60).to_string(),
        // An hour or more: in hours and minutes, as it is said.
        (true, _) if minutes == 0 => format!("{hours}h"),
        (true, _) => format!("{hours}h{minutes:02}"),
        (false, 0) => format!("{}:{:02}", s / 60, s % 60),
        (false, _) => format!("{hours}:{minutes:02}:{:02}", s % 60),
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

    #[test]
    fn drift_and_the_wait_for_the_start_read_as_people_say_them() {
        assert_eq!(drift(0), "on time");
        assert_eq!(drift(-45), "on time");
        assert_eq!(drift(240), "4 min behind");
        assert_eq!(drift(-130), "2 min ahead");
        assert_eq!(
            starting("10:30", 12 * 60).as_deref(),
            Some("starts 10:30 · in 12 min")
        );
        assert_eq!(starting("10:30", 20).as_deref(), Some("starts 10:30 · now"));
        assert_eq!(
            starting("10:30", -300).as_deref(),
            Some("starts 10:30 · 5 min ago")
        );
        // Not all day: a plan from last week says nothing.
        assert_eq!(starting("10:30", 5 * 3600), None);
        assert_eq!(starting("10:30", -2 * 3600), None);
    }

    const fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn a_length_field_takes_a_clock_or_hours_and_minutes() {
        for ok in [
            "", "4", "4:", "4:3", "4:30", ":30", "1:02:03", "3h", "3h15", "3h 15m", "45m",
        ] {
            assert!(typeable(ok), "{ok}");
        }
        for no in ["a", "4a", "4:3:0:1", "4.5", "-4", "3hh", "3h:15", "3m5m"] {
            assert!(!typeable(no), "{no}");
        }
    }

    #[test]
    fn hours_read_as_they_are_said() {
        assert_eq!(parse("3h"), Some(Some(secs(3 * 3600))));
        assert_eq!(parse("3h15"), Some(Some(secs(3 * 3600 + 15 * 60))));
        assert_eq!(parse("3H 15m"), Some(Some(secs(3 * 3600 + 15 * 60))));
        assert_eq!(parse("45m"), Some(Some(secs(45 * 60))));
        assert_eq!(parse("1:02:03"), Some(Some(secs(3723))));
        assert_eq!(parse("1:75:00"), None);
        assert_eq!(parse("h"), None, "still typing");
        // Without an h, a number is minutes and a colon is minutes and
        // seconds, as before.
        assert_eq!(parse("195"), Some(Some(secs(195 * 60))));
        assert_eq!(parse("3:15"), Some(Some(secs(195))));
        // And they are written back the way they read.
        assert_eq!(typed(secs(3 * 3600 + 15 * 60)), "3h15");
        assert_eq!(typed(secs(3 * 3600)), "3h");
        assert_eq!(typed(secs(3723)), "1:02:03");
        for text in ["3h15", "3h", "1:02:03", "45"] {
            assert_eq!(typed(parse(text).unwrap().unwrap()), text, "round trip");
        }
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
