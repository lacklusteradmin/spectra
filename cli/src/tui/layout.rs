//! kitty's layouts: where a tab's windows sit, and the one-cell borders
//! between them. Pure geometry, so every arrangement is tested without a
//! terminal.

use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// The first window on the left, the others stacked on the right.
    Tall,
    /// The first window on top, the others side by side beneath it.
    Fat,
    Grid,
    Horizontal,
    Vertical,
    /// Only the active window, at the full size.
    Stack,
}

impl Layout {
    const CYCLE: [Self; 6] = [
        Self::Tall,
        Self::Fat,
        Self::Grid,
        Self::Horizontal,
        Self::Vertical,
        Self::Stack,
    ];

    pub fn next(self) -> Self {
        let at = Self::CYCLE.iter().position(|layout| *layout == self);
        Self::CYCLE[at.map_or(0, |at| (at + 1) % Self::CYCLE.len())]
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Tall => "tall",
            Self::Fat => "fat",
            Self::Grid => "grid",
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
            Self::Stack => "stack",
        }
    }

    /// One entry per window: its place in `area`, or `None` when this layout
    /// does not show it.
    pub fn arrange(self, area: Rect, count: usize, active: usize) -> Vec<Option<Rect>> {
        if self == Self::Stack || count < 2 {
            return (0..count)
                .map(|index| (index == active).then_some(area))
                .collect();
        }
        let rects = match self {
            Self::Tall => {
                let sides = columns(area, 2);
                [vec![sides[0]], rows(sides[1], count - 1)].concat()
            }
            Self::Fat => {
                let sides = rows(area, 2);
                [vec![sides[0]], columns(sides[1], count - 1)].concat()
            }
            Self::Grid => grid(area, count),
            Self::Horizontal => columns(area, count),
            Self::Vertical | Self::Stack => rows(area, count),
        };
        rects.into_iter().map(Some).collect()
    }
}

/// `count` runs along `length`, one cell apart, sharing what is left over
/// from the first run on.
fn runs(start: u16, length: u16, count: usize) -> Vec<(u16, u16)> {
    let count_cells = u16::try_from(count).unwrap_or(u16::MAX).max(1);
    let usable = length.saturating_sub(count_cells - 1);
    let (each, extra) = (usable / count_cells, usable % count_cells);
    let mut at = start;
    (0..count_cells)
        .map(|index| {
            let run = each + u16::from(index < extra);
            let place = (at, run);
            at = at.saturating_add(run).saturating_add(1);
            place
        })
        .collect()
}

fn columns(area: Rect, count: usize) -> Vec<Rect> {
    runs(area.x, area.width, count)
        .into_iter()
        .map(|(x, width)| Rect { x, width, ..area })
        .collect()
}

fn rows(area: Rect, count: usize) -> Vec<Rect> {
    runs(area.y, area.height, count)
        .into_iter()
        .map(|(y, height)| Rect { y, height, ..area })
        .collect()
}

fn grid(area: Rect, count: usize) -> Vec<Rect> {
    let across = count.isqrt() + usize::from(count.isqrt().pow(2) < count);
    columns(area, across)
        .into_iter()
        .enumerate()
        .flat_map(|(column, place)| {
            let down = count / across + usize::from(column < count % across);
            rows(place, down)
        })
        .collect()
}

/// A cell of the border between windows.
#[derive(Debug, PartialEq, Eq)]
pub struct BorderCell {
    pub x: u16,
    pub y: u16,
    pub symbol: &'static str,
    /// Touches the active window, so it takes the accent colour.
    pub active: bool,
}

/// What no window covers is border. A lone window has none, as in kitty.
pub fn borders(area: Rect, rects: &[Option<Rect>], active: usize) -> Vec<BorderCell> {
    let shown: Vec<Rect> = rects.iter().flatten().copied().collect();
    let is_border = |x: i32, y: i32| {
        let inside = |rect: &Rect| {
            let (left, top) = (i32::from(rect.x), i32::from(rect.y));
            x >= left
                && x < left + i32::from(rect.width)
                && y >= top
                && y < top + i32::from(rect.height)
        };
        inside(&area) && !shown.iter().any(inside)
    };
    let around = rects.get(active).copied().flatten().map(|rect| Rect {
        x: rect.x.saturating_sub(1),
        y: rect.y.saturating_sub(1),
        width: rect.width + 1 + u16::from(rect.x > 0),
        height: rect.height + 1 + u16::from(rect.y > 0),
    });
    let mut cells = Vec::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let (wide_x, wide_y) = (i32::from(x), i32::from(y));
            if !is_border(wide_x, wide_y) {
                continue;
            }
            let symbol = junction(
                is_border(wide_x, wide_y - 1),
                is_border(wide_x, wide_y + 1),
                is_border(wide_x - 1, wide_y),
                is_border(wide_x + 1, wide_y),
            );
            let active = around.is_some_and(|rect| {
                x >= rect.left() && x < rect.right() && y >= rect.top() && y < rect.bottom()
            });
            cells.push(BorderCell {
                x,
                y,
                symbol,
                active,
            });
        }
    }
    cells
}

fn junction(up: bool, down: bool, left: bool, right: bool) -> &'static str {
    match (up, down, left, right) {
        (true, true, true, true) => "┼",
        (true, true, true, false) => "┤",
        (true, true, false, true) => "├",
        (true, false, true, true) => "┴",
        (false, true, true, true) => "┬",
        (true, false, true, false) => "┘",
        (true, false, false, true) => "└",
        (false, true, true, false) => "┐",
        (false, true, false, true) => "┌",
        (false, false, true, _) | (false, false, _, true) => "─",
        _ => "│",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 81,
        height: 25,
    };

    fn places(layout: Layout, count: usize) -> Vec<Rect> {
        layout
            .arrange(AREA, count, 0)
            .into_iter()
            .flatten()
            .collect()
    }

    #[test]
    fn every_layout_cycles_back_to_the_first() {
        let mut layout = Layout::Tall;
        for _ in 0..Layout::CYCLE.len() {
            layout = layout.next();
        }
        assert_eq!(layout, Layout::Tall);
    }

    #[test]
    fn a_lone_window_fills_the_area_without_a_border() {
        for layout in Layout::CYCLE {
            let rects = layout.arrange(AREA, 1, 0);
            assert_eq!(rects, vec![Some(AREA)]);
            assert!(borders(AREA, &rects, 0).is_empty());
        }
    }

    #[test]
    fn tall_puts_the_first_window_beside_the_stacked_rest() {
        let rects = places(Layout::Tall, 3);
        assert_eq!(rects[0], Rect::new(0, 0, 40, 25));
        assert_eq!(rects[1], Rect::new(41, 0, 40, 12));
        assert_eq!(rects[2], Rect::new(41, 13, 40, 12));
    }

    #[test]
    fn fat_puts_the_first_window_above_the_rest() {
        let rects = places(Layout::Fat, 3);
        assert_eq!(rects[0], Rect::new(0, 0, 81, 12));
        assert_eq!(rects[1], Rect::new(0, 13, 40, 12));
        assert_eq!(rects[2], Rect::new(41, 13, 40, 12));
    }

    #[test]
    fn stack_shows_only_the_active_window() {
        let rects = Layout::Stack.arrange(AREA, 3, 1);
        assert_eq!(rects, vec![None, Some(AREA), None]);
    }

    #[test]
    fn no_arrangement_overlaps_or_leaves_the_area() {
        for layout in Layout::CYCLE {
            for count in 1..=9 {
                let rects = places(layout, count);
                for (index, rect) in rects.iter().enumerate() {
                    assert_eq!(AREA.union(*rect), AREA, "{layout:?} {count}");
                    for other in &rects[index + 1..] {
                        assert!(!rect.intersects(*other), "{layout:?} {count}");
                    }
                }
            }
        }
    }

    #[test]
    fn grid_holds_every_window() {
        for count in 2..=9 {
            assert_eq!(places(Layout::Grid, count).len(), count);
        }
    }

    #[test]
    fn a_window_too_small_to_hold_a_cell_does_not_panic() {
        let tiny = Rect::new(0, 0, 3, 2);
        for layout in Layout::CYCLE {
            let rects = layout.arrange(tiny, 9, 0);
            let _ = borders(tiny, &rects, 0);
        }
    }

    #[test]
    fn the_border_meets_in_junctions_and_marks_the_active_window() {
        let rects = Layout::Tall.arrange(AREA, 3, 0);
        let cells = borders(AREA, &rects, 1);
        let at = |x, y| cells.iter().find(|cell| cell.x == x && cell.y == y);
        assert_eq!(at(40, 0).map(|cell| cell.symbol), Some("│"));
        assert_eq!(at(40, 12).map(|cell| cell.symbol), Some("├"));
        assert_eq!(at(60, 12).map(|cell| cell.symbol), Some("─"));
        assert!(at(60, 12).is_some_and(|cell| cell.active), "under window 2");
        assert!(at(40, 5).is_some_and(|cell| cell.active), "beside window 2");
        assert!(at(40, 20).is_some_and(|cell| !cell.active), "not beside it");
    }
}
