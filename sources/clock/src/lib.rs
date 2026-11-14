//! Things that show time on stage: the time of day, a timer counting up,
//! and countdowns, for a length or to a time of day.
//!
//! Each is a [`Kind::Text`] resource whose meta [`CLOCK`] says what it
//! counts ([`Clock`] reads it back). The text changes every second, so the
//! app draws it from that and from when it went on stage; the resource's
//! own inline text is only its name, for anything that doesn't know.
//!
//! The usual ones ("Clock", "Timer", a few countdowns) are scanned, so a
//! plain search finds them. Behind the bang, `!t 7` is a seven-minute
//! countdown, `!t 2:30` one of two and a half minutes, and also one to
//! half past two.

use std::time::Duration;

use lantern_resource::{
    Capabilities, Kind, Location, Query, Resource, ResourceId, Source, SourceError, SourceId,
};

/// The meta key that says what a resource counts: `now`, `up`,
/// `down:<seconds>` or `until:<minutes after midnight>`.
pub const CLOCK: &str = "clock";

/// What a clock resource shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clock {
    /// The time of day.
    Now,
    /// How long since it went on stage.
    Up,
    /// What's left of this long, from when it went on stage.
    Down(Duration),
    /// What's left until this time of day, as time since midnight.
    Until(Duration),
}

impl Clock {
    /// What `resource` counts, if it's one of these.
    pub fn of(resource: &Resource) -> Option<Clock> {
        let spec = resource.meta.get(CLOCK)?;
        match spec.split_once(':') {
            None if spec == "now" => Some(Clock::Now),
            None if spec == "up" => Some(Clock::Up),
            Some(("down", seconds)) => {
                Some(Clock::Down(Duration::from_secs(seconds.parse().ok()?)))
            }
            Some(("until", minutes)) => {
                let minutes: u64 = minutes.parse().ok()?;
                (minutes < 24 * 60).then(|| Clock::Until(Duration::from_secs(minutes * 60)))
            }
            _ => None,
        }
    }

    fn spec(self) -> String {
        match self {
            Clock::Now => "now".into(),
            Clock::Up => "up".into(),
            Clock::Down(length) => format!("down:{}", length.as_secs()),
            Clock::Until(time) => format!("until:{}", time.as_secs() / 60),
        }
    }

    /// What it says: `on_stage` is how long it has been up (`None` before
    /// it is, when it shows where it would start), `of_day` the time of
    /// day now as time since midnight.
    pub fn text(self, on_stage: Option<Duration>, of_day: Duration) -> String {
        const DAY: u64 = 24 * 60 * 60;
        match self {
            Clock::Now => {
                let minutes = of_day.as_secs() / 60;
                format!("{}:{:02}", minutes / 60 % 24, minutes % 60)
            }
            Clock::Up => span(on_stage.unwrap_or_default().as_secs()),
            Clock::Down(length) => {
                // Rounded up, so it reads 5:00 for its whole first second
                // and 0:00 only once it's over.
                let left = length.saturating_sub(on_stage.unwrap_or_default());
                span(left.as_secs() + u64::from(left.subsec_nanos() > 0))
            }
            Clock::Until(time) => {
                // Over the day's end: at 23:50, "until 0:10" is 20 minutes
                // away. More than half a day away, it has passed.
                let left = (time.as_secs() + DAY - of_day.as_secs() % DAY) % DAY;
                span(if left > DAY / 2 { 0 } else { left })
            }
        }
    }

    /// How long it takes, for a plan to time it: a countdown's length.
    pub fn length(self) -> Option<Duration> {
        match self {
            Clock::Down(length) => Some(length),
            _ => None,
        }
    }
}

/// `4:05`, or `1:02:03` from an hour up.
fn span(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    match h {
        0 => format!("{m}:{s:02}"),
        h => format!("{h}:{m:02}:{s:02}"),
    }
}

/// The source of clocks, timers and countdowns. Its bang is `t`.
pub struct ClockSource {
    id: SourceId,
}

impl ClockSource {
    /// A clock source with this id.
    pub fn new(id: SourceId) -> Self {
        Self { id }
    }

    fn resource(&self, clock: Clock) -> Resource {
        let (title, about) = match clock {
            Clock::Now => ("Clock".to_string(), "the time of day".to_string()),
            Clock::Up => (
                "Timer".to_string(),
                "counts up from when it's shown".to_string(),
            ),
            Clock::Down(length) => (
                format!("Countdown {}", said(length)),
                "counts down from when it's shown".to_string(),
            ),
            Clock::Until(time) => {
                let at = Clock::Now.text(None, time);
                (format!("Countdown to {at}"), format!("counts down to {at}"))
            }
        };
        let spec = clock.spec();
        let mut resource = Resource::new(
            ResourceId::new(&self.id, &spec),
            title.clone(),
            Kind::Text,
            Location::Inline(title),
        )
        .with_subtitle(about)
        .with_tag("time")
        .with_tag("tiempo")
        .with_meta(CLOCK, spec);
        if let Some(length) = clock.length() {
            resource = resource.with_meta(Resource::DURATION, length.as_secs().to_string());
        }
        resource
    }

    /// What `text` asks for: `7` (minutes), `2:30` (a length, and a time
    /// of day), with or without "countdown" before it. Nothing for
    /// anything else.
    fn asked(&self, text: &str) -> Vec<Resource> {
        let text = text.trim().to_lowercase();
        let number = [
            "countdown to",
            "countdown",
            "cuenta regresiva",
            "timer",
            "min",
        ]
        .iter()
        .fold(text.as_str(), |rest, word| {
            rest.trim().trim_start_matches(word).trim_end_matches(word)
        })
        .trim();
        match number.split_once(':') {
            None => match number.parse::<u64>() {
                Ok(minutes @ 1..=600) => {
                    vec![self.resource(Clock::Down(Duration::from_secs(minutes * 60)))]
                }
                _ => Vec::new(),
            },
            Some((first, second)) if second.len() == 2 => {
                let (Ok(first), Ok(second)) = (first.parse::<u64>(), second.parse::<u64>()) else {
                    return Vec::new();
                };
                if second >= 60 || first > 600 {
                    return Vec::new();
                }
                let mut found = Vec::new();
                if first * 60 + second > 0 {
                    found
                        .push(self.resource(Clock::Down(Duration::from_secs(first * 60 + second))));
                }
                if first < 24 {
                    found.push(self.resource(Clock::Until(Duration::from_secs(
                        (first * 60 + second) * 60,
                    ))));
                }
                found
            }
            Some(_) => Vec::new(),
        }
    }
}

/// A length as it's said: `5 min`, `2:30`, `1 h 30 min`.
fn said(length: Duration) -> String {
    let seconds = length.as_secs();
    match (seconds / 3600, seconds % 3600 / 60, seconds % 60) {
        (0, m, 0) => format!("{m} min"),
        (h, 0, 0) => format!("{h} h"),
        (h, m, 0) => format!("{h} h {m} min"),
        _ => span(seconds),
    }
}

impl Source for ClockSource {
    fn id(&self) -> &SourceId {
        &self.id
    }

    fn name(&self) -> &str {
        "Time"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            scan: true,
            search: true,
            on_request: false,
        }
    }

    fn bangs(&self) -> Vec<&str> {
        vec!["t"]
    }

    fn scan(&self) -> Result<Vec<Resource>, SourceError> {
        let minutes = |m: u64| Clock::Down(Duration::from_secs(m * 60));
        Ok([
            Clock::Now,
            Clock::Up,
            minutes(1),
            minutes(5),
            minutes(10),
            minutes(15),
        ]
        .into_iter()
        .map(|clock| self.resource(clock))
        .collect())
    }

    fn search(&self, query: &Query) -> Result<Vec<Resource>, SourceError> {
        // A bare number means a countdown only behind the bang: anywhere
        // else it's a hymn's number or a chapter.
        let banged = query.sources.contains(&self.id);
        let worded = ["countdown", "cuenta", "timer"]
            .iter()
            .any(|word| query.text.to_lowercase().contains(word));
        Ok(match banged || worded {
            true => self.asked(&query.text),
            false => Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn source() -> ClockSource {
        ClockSource::new(SourceId::new("clock").unwrap())
    }

    fn of_day(hours: u64, minutes: u64) -> Duration {
        Duration::from_secs((hours * 60 + minutes) * 60)
    }

    #[test]
    fn each_says_its_time() {
        let now = of_day(10, 52);
        assert_eq!(Clock::Now.text(None, now), "10:52");
        assert_eq!(Clock::Now.text(None, of_day(9, 5)), "9:05");
        assert_eq!(Clock::Up.text(None, now), "0:00");
        assert_eq!(Clock::Up.text(Some(Duration::from_secs(125)), now), "2:05");
        assert_eq!(Clock::Up.text(Some(61 * MIN), now), "1:01:00");
        let five = Clock::Down(5 * MIN);
        assert_eq!(five.text(None, now), "5:00", "where it would start");
        assert_eq!(five.text(Some(Duration::from_millis(400)), now), "5:00");
        assert_eq!(five.text(Some(Duration::from_millis(1400)), now), "4:59");
        assert_eq!(
            five.text(Some(5 * MIN - Duration::from_millis(1)), now),
            "0:01"
        );
        assert_eq!(five.text(Some(9 * MIN), now), "0:00", "it stops at nothing");
        let half_ten = Clock::Until(of_day(10, 30));
        assert_eq!(half_ten.text(None, of_day(10, 18)), "12:00");
        assert_eq!(half_ten.text(None, of_day(8, 0)), "2:30:00");
        assert_eq!(half_ten.text(None, of_day(10, 45)), "0:00", "it has passed");
        // Over midnight.
        assert_eq!(
            Clock::Until(of_day(0, 10)).text(None, of_day(23, 50)),
            "20:00"
        );
    }

    #[test]
    fn a_resource_reads_back_what_it_counts() {
        let source = source();
        for clock in [
            Clock::Now,
            Clock::Up,
            Clock::Down(90 * MIN),
            Clock::Until(of_day(19, 30)),
        ] {
            let resource = source.resource(clock);
            assert_eq!(Clock::of(&resource), Some(clock), "{}", resource.title);
            assert!(resource.id.belongs_to(source.id()));
            assert_eq!(resource.kind, Kind::Text);
        }
        let titles: Vec<String> = source
            .scan()
            .unwrap()
            .into_iter()
            .map(|r| r.title)
            .collect();
        assert_eq!(
            titles,
            [
                "Clock",
                "Timer",
                "Countdown 1 min",
                "Countdown 5 min",
                "Countdown 10 min",
                "Countdown 15 min"
            ]
        );
        // A countdown has a length for the plan to time; a clock has none.
        let five = source.resource(Clock::Down(5 * MIN));
        assert_eq!(
            five.meta.get(Resource::DURATION).map(String::as_str),
            Some("300")
        );
        assert!(
            !source
                .resource(Clock::Now)
                .meta
                .contains_key(Resource::DURATION)
        );
        assert_eq!(
            source.resource(Clock::Until(of_day(19, 30))).title,
            "Countdown to 19:30"
        );
        assert_eq!(
            source.resource(Clock::Down(90 * MIN)).title,
            "Countdown 1 h 30 min"
        );
    }

    #[test]
    fn a_number_is_a_countdown_only_behind_the_bang() {
        let source = source();
        let titles = |query: Query| -> Vec<String> {
            source
                .search(&query)
                .unwrap()
                .into_iter()
                .map(|r| r.title)
                .collect()
        };
        let banged = |text: &str| Query::new(text).with_source(source.id().clone());
        assert_eq!(titles(banged("7")), ["Countdown 7 min"]);
        assert_eq!(
            titles(banged("2:30")),
            ["Countdown 2:30", "Countdown to 2:30"]
        );
        assert_eq!(
            titles(banged("19:30")),
            ["Countdown 19:30", "Countdown to 19:30"]
        );
        assert_eq!(
            titles(banged("45:00")),
            ["Countdown 45 min"],
            "not a time of day"
        );
        assert!(titles(banged("juan 3 16")).is_empty());
        assert!(titles(banged("0")).is_empty());
        assert!(titles(banged("2:3")).is_empty());
        // Without the bang a number is a hymn's, unless it says countdown.
        assert!(titles(Query::new("7")).is_empty());
        assert_eq!(titles(Query::new("countdown 7")), ["Countdown 7 min"]);
        assert_eq!(titles(Query::new("Countdown 7 min")), ["Countdown 7 min"]);
    }
}
