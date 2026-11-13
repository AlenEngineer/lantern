//! The colours, as Material colour roles (`primary`, `surface`,
//! `on_surface_variant`…), so the app can follow a desktop theme.
//!
//! `colors = "<file>"` in the config names a JSON file of roles, the format
//! [Matugen](https://github.com/InioX/matugen) templates write
//! (`{"primary": "#ffb2bf", "on_surface": "#f0dee0", …}`). Roles the file
//! lacks keep the built-in dark palette's. The file is read again whenever
//! it changes, so a new wallpaper recolours the dashboard while it runs.

use std::path::PathBuf;
use std::time::SystemTime;

use iced::Color;
use serde::{Deserialize, Deserializer};

/// The roles the dashboard uses. See `docs/DESIGN.md` for which goes where.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(default)]
pub struct Palette {
    #[serde(deserialize_with = "hex")]
    pub primary: Color,
    #[serde(deserialize_with = "hex")]
    pub on_primary: Color,
    #[serde(deserialize_with = "hex")]
    pub on_primary_container: Color,
    #[serde(deserialize_with = "hex")]
    pub tertiary: Color,
    #[serde(deserialize_with = "hex")]
    pub on_tertiary_container: Color,
    #[serde(deserialize_with = "hex")]
    pub error: Color,
    #[serde(deserialize_with = "hex")]
    pub surface: Color,
    #[serde(deserialize_with = "hex")]
    pub on_surface: Color,
    #[serde(deserialize_with = "hex")]
    pub on_surface_variant: Color,
    #[serde(deserialize_with = "hex")]
    pub surface_container: Color,
    #[serde(deserialize_with = "hex")]
    pub surface_container_highest: Color,
    #[serde(deserialize_with = "hex")]
    pub outline: Color,
    #[serde(deserialize_with = "hex")]
    pub outline_variant: Color,
}

impl Default for Palette {
    /// Material's baseline dark scheme.
    fn default() -> Self {
        Palette {
            primary: rgb(0xD0BCFF),
            on_primary: rgb(0x381E72),
            on_primary_container: rgb(0xEADDFF),
            tertiary: rgb(0xEFB8C8),
            on_tertiary_container: rgb(0xFFD8E4),
            error: rgb(0xF2B8B5),
            surface: rgb(0x141218),
            on_surface: rgb(0xE6E0E9),
            on_surface_variant: rgb(0xCAC4D0),
            surface_container: rgb(0x211F26),
            surface_container_highest: rgb(0x36343B),
            outline: rgb(0x938F99),
            outline_variant: rgb(0x49454F),
        }
    }
}

impl Palette {
    /// Parses a roles file. Unknown keys are ignored: Matugen writes many.
    pub fn parse(json: &str) -> Result<Palette, String> {
        serde_json::from_str(json).map_err(|err| err.to_string())
    }

    /// `color` at `alpha`, for the tinted fills: a selected row is
    /// `primary` at 22%, the hover veil `on_surface` at 12%.
    pub fn tint(color: Color, alpha: f32) -> Color {
        Color { a: alpha, ..color }
    }
}

const fn rgb(hex: u32) -> Color {
    Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn hex<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Color, D::Error> {
    let text = String::deserialize(deserializer)?;
    text.strip_prefix('#')
        .filter(|digits| digits.len() == 6)
        .and_then(|digits| u32::from_str_radix(digits, 16).ok())
        .map(rgb)
        .ok_or_else(|| serde::de::Error::custom(format!("not a #rrggbb colour: {text:?}")))
}

/// The roles file and when it was last read, to notice when it changes.
#[derive(Debug)]
pub struct Source {
    path: PathBuf,
    read: Option<SystemTime>,
}

impl Source {
    pub fn new(path: PathBuf) -> Source {
        Source { path, read: None }
    }

    /// The palette, if the file changed since the last call (or this is the
    /// first). `Err` says why it couldn't be read; the caller keeps the
    /// palette it has.
    pub fn changed(&mut self) -> Option<Result<Palette, String>> {
        let modified = std::fs::metadata(&self.path)
            .and_then(|meta| meta.modified())
            .ok();
        if self.read.is_some() && modified == self.read {
            return None;
        }
        self.read = modified;
        let fail = |err: &dyn std::fmt::Display| format!("{}: {err}", self.path.display());
        Some(
            std::fs::read_to_string(&self.path)
                .map_err(|err| fail(&err))
                .and_then(|json| Palette::parse(&json).map_err(|err| fail(&err))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_come_from_the_file_and_missing_ones_from_the_default() {
        let palette =
            Palette::parse(r##"{"primary": "#ffb2bf", "scrim": "#000000", "surface": "#191112"}"##)
                .unwrap();
        assert_eq!(palette.primary, Color::from_rgb8(0xff, 0xb2, 0xbf));
        assert_eq!(palette.surface, Color::from_rgb8(0x19, 0x11, 0x12));
        assert_eq!(palette.error, Palette::default().error);
    }

    #[test]
    fn a_bad_colour_is_an_error() {
        let err = Palette::parse(r#"{"primary": "pink"}"#).unwrap_err();
        assert!(err.contains("not a #rrggbb colour"), "{err}");
    }

    #[test]
    fn the_file_is_read_again_only_when_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("colors.json");
        std::fs::write(&path, r##"{"primary": "#112233"}"##).unwrap();
        let mut source = Source::new(path.clone());
        let first = source.changed().unwrap().unwrap();
        assert_eq!(first.primary, Color::from_rgb8(0x11, 0x22, 0x33));
        assert!(source.changed().is_none(), "unchanged");

        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::write(&path, r##"{"primary": "#445566"}"##).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        let second = source.changed().unwrap().unwrap();
        assert_eq!(second.primary, Color::from_rgb8(0x44, 0x55, 0x66));

        std::fs::remove_file(&path).unwrap();
        assert!(source.changed().unwrap().is_err(), "gone");
    }
}
