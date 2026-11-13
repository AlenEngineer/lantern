//! One line of text that ends in "…" when it doesn't fit. iced's own text
//! can only wrap or run on, so a long title was chopped mid-letter at the
//! edge of whatever clipped it.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::text::{self, Paragraph as _};
use iced::advanced::widget::{self, Tree, Widget};
use iced::alignment::Vertical;
use iced::{Color, Element, Font, Length, Pixels, Rectangle, Size, mouse};

/// One line of `content`, cut with "…" at the width it's given.
pub fn cut(content: impl Into<String>) -> Cut {
    Cut {
        content: content.into(),
        size: None,
        font: None,
        color: None,
    }
}

/// See [`cut`]. Takes the size, font and colour the way `text` does.
pub struct Cut {
    content: String,
    size: Option<f32>,
    font: Option<Font>,
    color: Option<Color>,
}

impl Cut {
    pub fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    pub fn font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

/// The longest start of `content` (whole characters, no trailing space)
/// that fits with "…" after it, by `fits`; all of it when that fits.
/// `fits` must hold for anything shorter than something that fits.
pub fn shorten(content: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(content) {
        return content.to_string();
    }
    let ends: Vec<usize> = content.char_indices().map(|(at, _)| at).collect();
    let with_dots = |chars: usize| format!("{}…", content[..ends[chars]].trim_end());
    // The most characters kept, between none and all but one.
    let (mut low, mut high) = (0, ends.len().saturating_sub(1));
    while low < high {
        let middle = (low + high).div_ceil(2);
        if fits(&with_dots(middle)) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    if ends.is_empty() {
        return String::new();
    }
    with_dots(low)
}

/// What a [`Cut`] laid out last, so an unchanged line isn't shaped again
/// on every update.
struct State<P> {
    laid_out: Option<(String, f32, f32, Font)>,
    paragraph: Option<P>,
}

fn line<P: text::Paragraph<Font = Font>>(content: &str, size: f32, font: Font) -> P {
    P::with_text(text::Text {
        content,
        bounds: Size::new(f32::INFINITY, f32::INFINITY),
        size: Pixels(size),
        line_height: text::LineHeight::default(),
        font,
        align_x: text::Alignment::Default,
        align_y: Vertical::Top,
        shaping: text::Shaping::Advanced,
        wrapping: text::Wrapping::None,
    })
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Cut
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
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let width = limits.max().width;
        let size = self.size.unwrap_or(renderer.default_size().0);
        let font = self.font.unwrap_or(renderer.default_font());
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();
        let unchanged = state.laid_out.as_ref().is_some_and(|(content, w, s, f)| {
            *content == self.content && *w == width && *s == size && *f == font
        });
        if !unchanged {
            let fits = |text: &str| {
                line::<Renderer::Paragraph>(text, size, font)
                    .min_bounds()
                    .width
                    <= width
            };
            let shown = shorten(&self.content, fits);
            state.paragraph = Some(line(&shown, size, font));
            state.laid_out = Some((self.content.clone(), width, size, font));
        }
        let bounds = state
            .paragraph
            .as_ref()
            .map(|paragraph| paragraph.min_bounds())
            .unwrap_or(Size::ZERO);
        layout::Node::new(limits.resolve(Length::Shrink, Length::Shrink, bounds))
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State<Renderer::Paragraph>>();
        let Some(paragraph) = &state.paragraph else {
            return;
        };
        let bounds = layout.bounds();
        renderer.fill_paragraph(
            paragraph,
            bounds.position(),
            self.color.unwrap_or(style.text_color),
            bounds.intersection(viewport).unwrap_or(bounds),
        );
    }
}

impl<'a, Message, Theme, Renderer> From<Cut> for Element<'a, Message, Theme, Renderer>
where
    Renderer: text::Renderer<Font = Font> + 'a,
{
    fn from(cut: Cut) -> Self {
        Element::new(cut)
    }
}

#[cfg(test)]
mod tests {
    use super::shorten;

    #[test]
    fn a_line_too_long_keeps_its_start_and_says_it_was_cut() {
        let up_to = |most: usize| move |text: &str| text.chars().count() <= most;
        assert_eq!(shorten("Himno 99", up_to(20)), "Himno 99");
        assert_eq!(shorten("Himno 99 Jamás podrá", up_to(9)), "Himno 99…");
        assert_eq!(
            shorten("Jamás podrá", up_to(5)),
            "Jamá…",
            "whole characters"
        );
        assert_eq!(shorten("Himno 99", up_to(0)), "…", "nothing fits");
        assert_eq!(shorten("", up_to(0)), "");
    }
}
