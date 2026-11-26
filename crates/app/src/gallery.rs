//! Tiles side by side, for the library's Grid and Shapes: even 16:9
//! tiles two to a row, or each tile at its picture's own shape, in rows
//! packed edge to edge as a photo gallery packs them.

use std::ops::Range;

/// How many even tiles a row of the Grid has.
pub const COLUMNS: usize = 2;

/// How tall a row of the Shapes is meant to be: about as tall as a Grid
/// tile's picture in a library of the usual width.
pub const ROW: f32 = 64.0;

/// The room a Shapes tile's foot keeps for its gaps, at its edges and
/// between its title and its size.
const FOOT: f32 = 12.0;
/// The least room a title is put in: a few letters and the dots after
/// them.
const TITLE: f32 = 32.0;

/// One row of tiles at their own shapes, all as tall.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Which tiles, by their place among those packed.
    pub tiles: Range<usize>,
    pub height: f32,
}

/// Packs tiles of `shapes`, each its width over its height, into rows
/// `width` wide with `gap` between, in their order. A row takes tiles
/// until, stretched to the full width, it would be no taller than
/// `target`. It then keeps its last tile, or leaves it to start the next
/// row, whichever puts its height nearer `target`, and is stretched to
/// the width. The last row, short of the width, stays `target` tall.
pub fn pack(shapes: &[f32], width: f32, target: f32, gap: f32) -> Vec<Row> {
    let height = |tiles: Range<usize>| {
        let gaps = gap * (tiles.len() - 1) as f32;
        (width - gaps) / shapes[tiles].iter().sum::<f32>()
    };
    let mut rows = Vec::new();
    let (mut start, mut end) = (0, 0);
    while end < shapes.len() {
        end += 1;
        let tall = height(start..end);
        if tall > target {
            continue;
        }
        let without = (end - start > 1).then(|| height(start..end - 1));
        if let Some(without) = without
            && without - target < target - tall
        {
            end -= 1;
        }
        rows.push(Row {
            tiles: start..end,
            height: height(start..end),
        });
        start = end;
    }
    if start < shapes.len() {
        rows.push(Row {
            tiles: start..shapes.len(),
            height: target,
        });
    }
    rows
}

/// Whether a tile `width` wide has room on its foot for its title,
/// beside its size `badge` wide (0 for a tile without one).
pub fn titled(width: f32, badge: f32) -> bool {
    width - badge - FOOT >= TITLE
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: f32 = 16.0 / 9.0;

    /// How wide `row` comes out, its gaps and all.
    fn across(shapes: &[f32], row: &Row, gap: f32) -> f32 {
        let tiles: f32 = shapes[row.tiles.clone()]
            .iter()
            .map(|shape| shape * row.height)
            .sum();
        tiles + gap * (row.tiles.len() - 1) as f32
    }

    #[test]
    fn full_rows_reach_the_width_and_the_last_keeps_its_height() {
        // The e2e folder: image, page, a PDF page, photo, a portrait, video.
        let shapes = [WIDE, WIDE, 0.75, WIDE, 0.8, WIDE];
        for width in [214.0, 230.0, 380.0] {
            let rows = pack(&shapes, width, ROW, 4.0);
            let (last, full) = rows.split_last().expect("some rows");
            for row in full {
                let across = across(&shapes, row, 4.0);
                assert!(
                    (across - width).abs() < 1.0,
                    "{width}: {row:?} is {across} wide"
                );
            }
            assert_eq!(last.height, ROW, "{width}: the last row isn't stretched");
            assert!(across(&shapes, last, 4.0) < width, "{width}: {last:?}");
            let order: Vec<usize> = rows.iter().flat_map(|row| row.tiles.clone()).collect();
            assert_eq!(order, (0..shapes.len()).collect::<Vec<_>>(), "{width}");
        }
    }

    #[test]
    fn a_row_keeps_its_last_tile_only_when_that_brings_it_nearer_its_height() {
        // Two squares come to 50 exactly: the second stays.
        assert_eq!(
            pack(&[1.0, 1.0, 1.0], 100.0, 50.0, 0.0),
            [
                Row {
                    tiles: 0..2,
                    height: 50.0
                },
                Row {
                    tiles: 2..3,
                    height: 50.0
                },
            ]
        );
        // A panorama would bring the row down to 14.5, further from 50
        // than 52.6 without it: it goes on alone, shrunk to the width.
        let rows = pack(&[1.0, 0.9, 5.0], 100.0, 50.0, 0.0);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].tiles, 0..2);
        assert!((rows[0].height - 100.0 / 1.9).abs() < 0.01, "{rows:?}");
        assert_eq!(rows[1].tiles, 2..3);
        assert!((rows[1].height - 20.0).abs() < 0.01, "{rows:?}");
    }

    #[test]
    fn nothing_packs_into_no_rows() {
        assert!(pack(&[], 230.0, ROW, 4.0).is_empty());
    }

    #[test]
    fn a_tile_too_narrow_for_its_title_goes_without() {
        let shapes = [WIDE, 0.5, WIDE];
        let rows = pack(&shapes, 230.0, ROW, 4.0);
        assert_eq!(rows.len(), 1, "{rows:?}");
        let widths: Vec<f32> = shapes.iter().map(|shape| shape * rows[0].height).collect();
        let size = 27.0;
        assert!(!titled(widths[1], 0.0), "a tall one, {} wide", widths[1]);
        assert!(
            titled(widths[0], size),
            "{} wide beside its size",
            widths[0]
        );
        assert!(titled(widths[2], size));
        assert!(
            !titled(widths[0], widths[0] - 20.0),
            "a size that takes most of it leaves no room"
        );
    }
}
