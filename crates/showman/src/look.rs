//! How the stage looks: what's behind the content, and how text is set.
//! A renderer draws a [`Look`]; this crate only describes it, so a plan can
//! carry one and look the same on any machine.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// How the stage looks. Sizes are in points on a stage 1080 lines tall, so
/// a smaller monitor of the same stage is a true miniature.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Look {
    /// The colour behind everything.
    pub background: Rgb,
    /// An image over the background colour, covering the stage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<PathBuf>,
    /// The colour of slide text.
    pub text: Rgb,
    /// A font family installed on the system; `None` is the renderer's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Text is set as large as fits, up to this.
    pub max_size: u16,
    /// ...and never smaller than this, even when it then overflows.
    pub min_size: u16,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            background: Rgb::BLACK,
            image: None,
            text: Rgb::WHITE,
            font: None,
            max_size: 96,
            min_size: 40,
        }
    }
}

impl Look {
    /// The height, in lines, that sizes are given for.
    pub const STAGE_HEIGHT: f32 = 1080.0;

    /// Whether nothing was changed from the default (black, white text).
    pub fn is_default(&self) -> bool {
        *self == Look::default()
    }

    /// The size range, smallest first, whichever way round it was set, and
    /// never below 1.
    pub fn sizes(&self) -> (u16, u16) {
        let (a, b) = (self.min_size.max(1), self.max_size.max(1));
        (a.min(b), a.max(b))
    }
}

/// A colour, written `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    /// `#000000`.
    pub const BLACK: Rgb = Rgb([0, 0, 0]);
    /// `#ffffff`.
    pub const WHITE: Rgb = Rgb([255, 255, 255]);
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b] = self.0;
        write!(f, "#{r:02x}{g:02x}{b:02x}")
    }
}

impl FromStr for Rgb {
    type Err = String;

    /// `#rrggbb` or `#rgb`, the `#` optional.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let hex = text.trim().trim_start_matches('#');
        let digits: Vec<u8> = hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect::<Option<_>>()
            .ok_or_else(|| format!("“{text}” isn't a colour like #1a2b3c"))?;
        match digits[..] {
            [r, g, b] => Ok(Rgb([r * 17, g * 17, b * 17])),
            [r1, r2, g1, g2, b1, b2] => Ok(Rgb([r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2])),
            _ => Err(format!("“{text}” isn't a colour like #1a2b3c")),
        }
    }
}

impl Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_read_long_and_short_and_write_long() {
        assert_eq!("#1a2B3c".parse(), Ok(Rgb([0x1a, 0x2b, 0x3c])));
        assert_eq!("fff".parse(), Ok(Rgb::WHITE));
        assert!("#12345".parse::<Rgb>().is_err());
        assert!("blue".parse::<Rgb>().is_err());
        assert_eq!(Rgb([0x1a, 0x2b, 0x3c]).to_string(), "#1a2b3c");
    }

    #[test]
    fn a_partial_look_fills_in_the_rest_and_sizes_come_out_in_order() {
        let look: Look =
            serde_json::from_str(r##"{"background": "#102030", "max_size": 30}"##).unwrap();
        assert_eq!(look.background, Rgb([0x10, 0x20, 0x30]));
        assert_eq!(look.text, Rgb::WHITE);
        assert_eq!(look.sizes(), (30, 40), "a max below the min swaps");
        assert!(Look::default().is_default());
        let json = serde_json::to_string(&Look::default()).unwrap();
        assert_eq!(
            json,
            r##"{"background":"#000000","text":"#ffffff","max_size":96,"min_size":40}"##
        );
    }
}
