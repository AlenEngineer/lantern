//! Slide text set as large as fits: the size is searched between the
//! look's limits, measured with the renderer's own text layout at the real
//! size of the box, so the stage and its monitors agree. Text that doesn't
//! fit even at the smallest size is split into more slides by a
//! [`Splitter`], which measures at the stage's shape.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::text::{self, Paragraph as _};
use iced::advanced::widget::{self, Tree, Widget};
use iced::alignment::Vertical;
use iced::{Color, Element, Font, Length, Pixels, Rectangle, Size, mouse};
use lantern_engine::{Look, Rgb, Scene, fit_frames};

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

/// The paragraph every measure and draw uses: centred, wrapping at words.
fn paragraph<P: text::Paragraph<Font = Font>>(
    content: &str,
    size: f32,
    width: f32,
    font: Font,
) -> P {
    P::with_text(text::Text {
        content,
        bounds: Size::new(width, f32::INFINITY),
        size: Pixels(size),
        line_height: text::LineHeight::Relative(1.25),
        font,
        align_x: text::Alignment::Center,
        align_y: Vertical::Center,
        shaping: text::Shaping::Advanced,
        wrapping: text::Wrapping::WordOrGlyph,
    })
}

/// The box text is set in, inside a stage of `bounds`.
fn inner(bounds: Size) -> Size {
    Size::new(
        bounds.width * (1.0 - 2.0 * MARGIN),
        bounds.height * (1.0 - 2.0 * MARGIN),
    )
}

/// Whether `content` fits `inner` at `size`.
fn fits<P: text::Paragraph<Font = Font>>(
    content: &str,
    size: f32,
    inner: Size,
    font: Font,
) -> bool {
    let measured = paragraph::<P>(content, size, inner.width, font).min_bounds();
    measured.height <= inner.height && measured.width <= inner.width + 1.0
}

/// Splits slide text so each slide fits the stage at the look's smallest
/// size (see [`fit_frames`]). Measures at the stage's shape, in the
/// 1080-line units sizes are given in, so the split is the same on the
/// stage and its monitors. Remembers each text's split until the look or
/// the stage's shape changes.
pub struct Splitter {
    look: Look,
    /// The stage's width over its height.
    aspect: f32,
    split: Mutex<HashMap<String, Arc<Vec<String>>>>,
}

impl Splitter {
    pub fn new(look: &Look) -> Splitter {
        Splitter {
            look: look.clone(),
            aspect: 16.0 / 9.0,
            split: Mutex::default(),
        }
    }

    /// Measures with `look` from now on. Returns whether it changed.
    pub fn set_look(&mut self, look: &Look) -> bool {
        if self.look == *look {
            return false;
        }
        self.look = look.clone();
        self.forget();
        true
    }

    /// Measures for a stage of `size` from now on. Returns whether its
    /// shape changed.
    pub fn set_stage(&mut self, size: Size) -> bool {
        let aspect = size.width / size.height;
        if !aspect.is_finite() || aspect <= 0.0 || (aspect - self.aspect).abs() < 0.001 {
            return false;
        }
        self.aspect = aspect;
        self.forget();
        true
    }

    fn forget(&mut self) {
        self.split
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// `body` as slides that fit.
    pub fn slides(&self, body: &str) -> Arc<Vec<String>> {
        let mut split = self.split.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(slides) = split.get(body) {
            return slides.clone();
        }
        let stage = Size::new(Look::STAGE_HEIGHT * self.aspect, Look::STAGE_HEIGHT);
        let (min, _) = self.look.sizes();
        let font = font(&self.look);
        let slides = Arc::new(fit_frames(body, |piece| {
            fits::<iced::advanced::graphics::text::Paragraph>(
                piece,
                f32::from(min),
                inner(stage),
                font,
            )
        }));
        split.insert(body.to_string(), slides.clone());
        slides
    }

    /// The scene's current slide, for inline text.
    pub fn slide(&self, scene: &Scene) -> Option<String> {
        let slides = self.slides(scene.resource.location.as_text()?);
        let last = slides.len().checked_sub(1)?;
        Some(slides[scene.frame().min(last)].clone())
    }
}

/// What a [`FitText`] laid out last: for which text, box and look, and
/// the paragraph it chose. Layout runs on every update, and searching the
/// size of a long text each time would stall the app, so an unchanged
/// layout reuses it.
struct State<P> {
    laid_out: Option<(String, Size, Look)>,
    paragraph: Option<P>,
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for FitText
where
    Renderer: text::Renderer<Font = Font>,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State<Renderer::Paragraph>>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::<Renderer::Paragraph> {
            laid_out: None,
            paragraph: None,
        })
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
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();
        let unchanged = state
            .laid_out
            .as_ref()
            .is_some_and(|(content, size, look)| {
                *content == self.content && *size == bounds && *look == self.look
            });
        if unchanged {
            return layout::Node::new(bounds);
        }
        let inner = inner(bounds);
        let scale = bounds.height / Look::STAGE_HEIGHT;
        let (min, max) = self.look.sizes();
        let font = font(&self.look);
        let content = self.content.as_str();
        let size = largest(f32::from(min) * scale, f32::from(max) * scale, |size| {
            fits::<Renderer::Paragraph>(content, size, inner, font)
        });
        let mut chosen = paragraph::<Renderer::Paragraph>(content, size, inner.width, font);
        chosen.resize(Size::new(
            inner.width,
            inner.height.max(chosen.min_bounds().height),
        ));
        state.paragraph = Some(chosen);
        state.laid_out = Some((self.content.clone(), bounds, self.look.clone()));
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
        let Some(paragraph) = &tree
            .state
            .downcast_ref::<State<Renderer::Paragraph>>()
            .paragraph
        else {
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

    #[test]
    fn a_text_too_long_for_the_stage_splits_and_a_short_one_stays_whole() {
        let look = Look::default();
        let mut splitter = Splitter::new(&look);
        assert_eq!(*splitter.slides("Cristo vive"), ["Cristo vive"]);
        let verse = "Porque de tal manera amó Dios al mundo, que ha dado á su Hijo unigénito, para que todo aquel que en él cree, no se pierda, mas tenga vida eterna. ";
        let long = verse.repeat(20);
        let slides = splitter.slides(&long);
        assert!(slides.len() > 1, "{slides:?}");
        assert!(
            slides.iter().all(|s| s.ends_with('.')),
            "cut at sentences: {slides:?}"
        );

        // A narrower stage needs at least as many slides.
        assert!(splitter.set_stage(Size::new(1024.0, 768.0)));
        assert!(splitter.slides(&long).len() >= slides.len());
        assert!(!splitter.set_stage(Size::new(2048.0, 1536.0)), "same shape");
    }
}
