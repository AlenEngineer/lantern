//! How big a picture or a screen is, counted in its own pixels, and
//! whether a picture is too small for the screen it is shown on.

use std::fmt;

/// A picture's or a screen's size, in its own pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
}

impl fmt::Display for Pixels {
    /// "1920×1080".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}×{}", self.width, self.height)
    }
}

/// Whether `picture` looks soft on `screen`: the stage must scale it up
/// to fit it there. The stage fits a picture inside its screen, as large
/// as it goes uncropped, so it is scaled up only when it is smaller than
/// the screen both ways. One as wide or as tall as the screen is shown
/// at its own pixels, or fewer.
pub fn looks_soft(picture: Pixels, screen: Pixels) -> bool {
    picture.width < screen.width && picture.height < screen.height
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(width: u32, height: u32) -> Pixels {
        Pixels { width, height }
    }

    #[test]
    fn a_picture_looks_soft_only_where_the_stage_scales_it_up() {
        let screen = pixels(1920, 1080);
        assert!(looks_soft(pixels(800, 600), screen), "smaller both ways");
        assert!(looks_soft(pixels(1919, 1079), screen), "a pixel short");
        assert!(!looks_soft(screen, screen), "the screen's own size");
        assert!(!looks_soft(pixels(3840, 2160), screen), "scaled down");
        assert!(
            !looks_soft(pixels(1920, 800), screen),
            "as wide as the screen, shown at its own pixels"
        );
        assert!(
            !looks_soft(pixels(1080, 1920), screen),
            "a portrait as tall as the screen is scaled down"
        );
        assert!(
            looks_soft(screen, pixels(3840, 2160)),
            "Full HD on a 4K screen"
        );
    }

    #[test]
    fn a_size_reads_width_by_height() {
        assert_eq!(pixels(800, 450).to_string(), "800×450");
    }
}
