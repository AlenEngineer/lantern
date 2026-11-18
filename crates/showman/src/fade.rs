use std::num::NonZeroU8;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How long a fade takes: whole seconds, one at the least. A fade that
/// takes no time is no fade, so there is no such length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FadeTime(NonZeroU8);

impl FadeTime {
    /// The longest a fade takes.
    pub const LONGEST: FadeTime = FadeTime(NonZeroU8::MAX);

    /// A fade of `seconds`, or of [`LONGEST`](Self::LONGEST) past it;
    /// `None` for none.
    pub fn from_secs(seconds: u64) -> Option<FadeTime> {
        NonZeroU8::new(u8::try_from(seconds).unwrap_or(u8::MAX)).map(FadeTime)
    }

    /// How many seconds it takes.
    pub fn secs(self) -> u64 {
        u64::from(self.0.get())
    }

    /// How long it takes.
    pub fn duration(self) -> Duration {
        Duration::from_secs(self.secs())
    }
}

/// What is on stage fading out as the plan goes on from the step that
/// put it there. The program fades: its sound, and its picture when it
/// has one. Anything else on program, or another slide, ends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FadeOut {
    /// Over how long, from when the stage was told.
    pub over: FadeTime,
    /// What is seen fades too. Not for a sound played over a picture
    /// that another step put up: that picture stays.
    pub picture: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_takes_whole_seconds_and_none_is_no_fade() {
        assert_eq!(FadeTime::from_secs(0), None);
        let three = FadeTime::from_secs(3).unwrap();
        assert_eq!(three.secs(), 3);
        assert_eq!(three.duration(), Duration::from_secs(3));
        assert_eq!(FadeTime::from_secs(1000), Some(FadeTime::LONGEST));
        assert_eq!(serde_json::to_string(&three).unwrap(), "3");
        assert!(serde_json::from_str::<FadeTime>("0").is_err());
    }
}
