//! Slide text set as large as fits: the size is searched between the
//! look's limits, measured with the renderer's own text layout at the real
//! size of the box, so the stage and its monitors agree.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::text::{self, Paragraph as _};
use iced::advanced::widget::{self, Tree, Widget};
use iced::alignment::Vertical;
use iced::{Color, Element, Font, Length, Pixels, Rectangle, Size, mouse};
use lantern_engine::{Look, Rgb};

/// The share of the box's width kept clear on each side, and of its height
/// above and below.
const MARGIN: f32 = 0.05;
/// How close the size search gets, in pixels.
const PRECISION: f32 = 0.5;

/// Slide text, as large as fits within `look`'s size range.
pub struct FitText {
    content: String,
    look: Look,
}

pub fn fit_text(content: impl Into<String>, look: &Look) -> FitText {
    FitText {
        content: content.into(),
        look: look.clone(),
    }
}

/// The colour `rgb` names, for drawing.
pub fn color(rgb: Rgb) -> Color {
    let [r, g, b] = rgb.0;
    Color::from_rgb8(r, g, b)
}

/// The font `look` names. Family names are kept for the life of the app,
/// since a font is named by a `'static` string; there are only a handful.
pub fn font(look: &Look) -> Font {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let Some(family) = look
        .font
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
    else {
        return Font::DEFAULT;
    };
    let mut names = NAMES.get_or_init(Default::default).lock().unwrap();
    let name = match names.get(family) {
        Some(name) => *name,
        None => {
            let name: &'static str = Box::leak(family.to_owned().into_boxed_str());
            names.insert(name);
            name
        }
    };
    Font::with_name(name)
}

/// The font families installed, as the renderer sees them, in name order.
pub fn families() -> Vec<String> {
    let Ok(mut system) = iced::advanced::graphics::text::font_system().write() else {
        return Vec::new();
    };
    let mut names: Vec<String> = system
        .raw()
        .db()
        .faces()
        .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    names
}

/// The largest size in `[min, max]` for which `fits` holds, or `min` when
/// none does. `fits` must hold for every size below one that it holds for.
pub fn largest(min: f32, max: f32, fits: impl Fn(f32) -> bool) -> f32 {
    if fits(max) {
        return max;
    }
    let (mut low, mut high) = (min, max);
    while high - low > PRECISION {
        let middle = (low + high) / 2.0;
        if fits(middle) {
            low = middle;
        } else {
            high = middle;
        }
    }
    low
}

struct State<P>(Option<P>);

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for FitText
where
    Renderer: text::Renderer<Font = Font>,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State<Renderer::Paragraph>>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::<Renderer::Paragraph>(None))
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let bounds = limits.max();
        let inner = Size::new(
            bounds.width * (1.0 - 2.0 * MARGIN),
            bounds.height * (1.0 - 2.0 * MARGIN),
        );
        let scale = bounds.height / Look::STAGE_HEIGHT;
        let (min, max) = self.look.sizes();
        let font = font(&self.look);
        let paragraph = |size: f32| {
            Renderer::Paragraph::with_text(text::Text {
                content: self.content.as_str(),
                bounds: Size::new(inner.width, f32::INFINITY),
                size: Pixels(size),
                line_height: text::LineHeight::Relative(1.25),
                font,
                align_x: text::Alignment::Center,
                align_y: Vertical::Center,
                shaping: text::Shaping::Advanced,
                wrapping: text::Wrapping::WordOrGlyph,
            })
        };
        let fits = |size: f32| {
            let measured = paragraph(size).min_bounds();
            measured.height <= inner.height && measured.width <= inner.width + 1.0
        };
        let size = largest(f32::from(min) * scale, f32::from(max) * scale, fits);
        let mut chosen = paragraph(size);
        chosen.resize(Size::new(
            inner.width,
            inner.height.max(chosen.min_bounds().height),
        ));
        tree.state.downcast_mut::<State<Renderer::Paragraph>>().0 = Some(chosen);
        layout::Node::new(bounds)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let Some(paragraph) = &tree.state.downcast_ref::<State<Renderer::Paragraph>>().0 else {
            return;
        };
        let bounds = layout.bounds();
        let anchor = bounds.anchor(
            paragraph.min_bounds(),
            paragraph.align_x(),
            paragraph.align_y(),
        );
        renderer.fill_paragraph(
            paragraph,
            anchor,
            color(self.look.text),
            bounds.intersection(viewport).unwrap_or(bounds),
        );
    }
}

impl<'a, Message, Theme, Renderer> From<FitText> for Element<'a, Message, Theme, Renderer>
where
    Renderer: text::Renderer<Font = Font> + 'a,
{
    fn from(text: FitText) -> Self {
        Element::new(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_largest_size_that_fits_is_found_between_the_limits() {
        assert_eq!(
            largest(10.0, 50.0, |_| true),
            50.0,
            "short text takes the max"
        );
        assert_eq!(
            largest(10.0, 50.0, |_| false),
            10.0,
            "long text stops at the min"
        );
        let size = largest(10.0, 50.0, |s| s <= 31.0);
        assert!((30.5..=31.0).contains(&size), "{size}");
    }

    #[test]
    fn a_font_is_named_by_its_family_and_blank_is_the_default() {
        let mut look = Look::default();
        assert_eq!(font(&look), Font::DEFAULT);
        look.font = Some(" DejaVu Serif ".into());
        assert_eq!(font(&look), Font::with_name("DejaVu Serif"));
        look.font = Some("".into());
        assert_eq!(font(&look), Font::DEFAULT);
    }
}
