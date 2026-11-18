//! A row that takes a moment to come to its height: as it grows or
//! shrinks, the rows under it slide rather than jump.
//!
//! Each row's height is kept by what it stands for, not where it is in
//! the list, so a row that moves to another place carries on from the
//! height it had there.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;
use std::time::Instant;

use iced::advanced::widget::{self, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector, mouse, window};

use crate::motion::{Glide, ROWS};

/// How tall each row is shown, on its way to its own height.
pub type Heights<K> = Rc<RefCell<HashMap<K, Glide>>>;

/// `content`, shown as tall as `heights` says for `key` while that
/// moves to its own height. One not seen before is its height at once,
/// unless it `grows` in from nothing. `still`, nothing moves.
pub fn settle<'a, K, Message: 'a>(
    key: K,
    heights: &Heights<K>,
    grows: bool,
    still: bool,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message>
where
    K: Copy + Eq + Hash + 'a,
{
    Element::new(Settle {
        key,
        heights: heights.clone(),
        grows,
        still,
        content: content.into(),
    })
}

struct Settle<'a, K, Message> {
    key: K,
    heights: Heights<K>,
    grows: bool,
    still: bool,
    content: Element<'a, Message>,
}

impl<K, Message> Widget<Message, Theme, Renderer> for Settle<'_, K, Message>
where
    K: Copy + Eq + Hash,
{
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.content.as_widget().size().width, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let child = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        let own = child.size().height;
        let now = Instant::now();
        let mut heights = self.heights.borrow_mut();
        let glide = heights
            .entry(self.key)
            .or_insert_with(|| Glide::at(if self.grows { 0.0 } else { own }, now));
        if glide.to != own {
            *glide = match self.still {
                true => Glide::at(own, now),
                false => glide.towards(own, now, ROWS, true),
            };
        }
        let shown = glide.shown(now);
        layout::Node::with_children(Size::new(child.size().width, shown), vec![child])
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        if let Some(child) = layout.children().next() {
            self.content
                .as_widget_mut()
                .operate(&mut tree.children[0], child, renderer, operation);
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let Event::Window(window::Event::RedrawRequested(now)) = event
            && self
                .heights
                .borrow()
                .get(&self.key)
                .is_some_and(|glide| glide.moving(*now))
        {
            shell.invalidate_layout();
            shell.request_redraw();
        }
        let Some(child) = layout.children().next() else {
            return;
        };
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            child,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let (Some(child), true) = (layout.children().next(), cursor.is_over(layout.bounds()))
        else {
            return mouse::Interaction::None;
        };
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            child,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let Some(child) = layout.children().next() else {
            return;
        };
        let bounds = layout.bounds();
        let draw = |renderer: &mut Renderer| {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                child,
                cursor,
                viewport,
            );
        };
        // Shorter than itself, what is below its shown height is cut off.
        match child.bounds().height > bounds.height {
            true => {
                if let Some(seen) = bounds.intersection(viewport) {
                    iced::advanced::Renderer::with_layer(renderer, seen, draw);
                }
            }
            false => draw(renderer),
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let child = layout.children().next()?;
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            child,
            renderer,
            viewport,
            translation,
        )
    }
}
