//! Pane frames: Herdr draws borders only when a tab shows several panes, joins the
//! lines into box-drawing junctions, and highlights the focused pane's edges.
//! Adapted from Herdr `src/ui/panes.rs`, including `pane_borders`, `pane_outer_borders`,
//! and `pane_gaps`.
use super::{chrome::truncate_end, palette::Palette};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Block, Borders},
};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub rect: Rect,
    pub borders: Borders,
    pub focused: bool,
    pub label: Option<String>,
}

/// The cells inside a pane's frame.
pub fn inner(rect: Rect, borders: Borders) -> Rect {
    if borders.is_empty() {
        rect
    } else {
        Block::default().borders(borders).inner(rect)
    }
}

fn ranges_overlap(a_start: u16, a_len: u16, b_start: u16, b_len: u16) -> bool {
    a_start < b_start.saturating_add(b_len) && b_start < a_start.saturating_add(a_len)
}

fn has_right_neighbor(rect: Rect, others: &[Rect]) -> bool {
    others.iter().any(|other| {
        *other != rect
            && other.x == rect.right()
            && ranges_overlap(rect.y, rect.height, other.y, other.height)
    })
}

fn has_neighbor_below(rect: Rect, others: &[Rect]) -> bool {
    others.iter().any(|other| {
        *other != rect
            && other.y == rect.bottom()
            && ranges_overlap(rect.x, rect.width, other.x, other.width)
    })
}

/// Herdr's `apply_pane_chrome`: which edges each pane draws, and the one-cell gap
/// between borderless split panes. Returns each rect with its borders.
pub fn apply_chrome(
    rects: &[Rect],
    pane_borders: crate::preferences::PaneBorders,
    pane_gaps: bool,
    pane_outer_borders: bool,
) -> Vec<(Rect, Borders)> {
    let multi_pane = rects.len() > 1;
    let bordered = pane_borders.shows_borders(multi_pane);
    let outer_left = rects.iter().map(|r| r.x).min().unwrap_or(0);
    let outer_top = rects.iter().map(|r| r.y).min().unwrap_or(0);
    let outer_right = rects.iter().map(|r| r.right()).max().unwrap_or(0);
    let outer_bottom = rects.iter().map(|r| r.bottom()).max().unwrap_or(0);
    rects
        .iter()
        .map(|&original| {
            let mut rect = original;
            let right = multi_pane && has_right_neighbor(original, rects);
            let below = multi_pane && has_neighbor_below(original, rects);
            if multi_pane && pane_gaps && !pane_borders.draws_borders() {
                if right && rect.width > 1 {
                    rect.width -= 1;
                }
                if below && rect.height > 1 {
                    rect.height -= 1;
                }
            }
            let borders = if !bordered {
                Borders::NONE
            } else {
                let mut borders = Borders::ALL;
                if !pane_gaps {
                    if right {
                        borders.remove(Borders::RIGHT);
                    }
                    if below {
                        borders.remove(Borders::BOTTOM);
                    }
                }
                if !pane_outer_borders {
                    if rect.x == outer_left {
                        borders.remove(Borders::LEFT);
                    }
                    if rect.y == outer_top {
                        borders.remove(Borders::TOP);
                    }
                    if rect.right() == outer_right {
                        borders.remove(Borders::RIGHT);
                    }
                    if rect.bottom() == outer_bottom {
                        borders.remove(Borders::BOTTOM);
                    }
                }
                borders
            };
            (rect, borders)
        })
        .collect()
}

/// The terminal's cells, leaving Herdr's scrollbar gutter outside the alternate screen.
pub fn terminal_rect(inner: Rect, alternate_screen: bool) -> (Rect, Option<Rect>) {
    if alternate_screen || inner.width <= 4 {
        return (inner, None);
    }
    (
        Rect::new(inner.x, inner.y, inner.width - 1, inner.height),
        Some(Rect::new(inner.right() - 1, inner.y, 1, inner.height)),
    )
}

#[derive(Clone, Copy, Default)]
struct LineCell {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

fn symbol(line: LineCell) -> &'static str {
    match (line.up, line.down, line.left, line.right) {
        (true, true, true, true) => "┼",
        (true, true, true, false) => "┤",
        (true, true, false, true) => "├",
        (true, false, true, true) => "┴",
        (false, true, true, true) => "┬",
        (true, true, false, false) | (true, false, false, false) | (false, true, false, false) => {
            "│"
        }
        (false, false, true, true) | (false, false, true, false) | (false, false, false, true) => {
            "─"
        }
        (false, true, false, true) => "┌",
        (false, true, true, false) => "┐",
        (true, false, false, true) => "└",
        (true, false, true, false) => "┘",
        _ => "",
    }
}

fn own_border(x: u16, y: u16, rect: Rect, borders: Borders) -> bool {
    if rect.is_empty() {
        return false;
    }
    let right = rect.right() - 1;
    let bottom = rect.bottom() - 1;
    let in_rows = y >= rect.y && y <= bottom;
    let in_cols = x >= rect.x && x <= right;
    (in_rows
        && ((x == rect.x && borders.contains(Borders::LEFT))
            || (x == right && borders.contains(Borders::RIGHT))))
        || (in_cols
            && ((y == rect.y && borders.contains(Borders::TOP))
                || (y == bottom && borders.contains(Borders::BOTTOM))))
}

pub fn render_borders(buffer: &mut Buffer, frames: &[Frame], palette: &Palette) {
    let mut cells = HashMap::<(u16, u16), LineCell>::new();
    for frame in frames
        .iter()
        .filter(|f| !f.borders.is_empty() && !f.rect.is_empty())
    {
        let rect = frame.rect;
        let right = rect.right() - 1;
        let bottom = rect.bottom() - 1;
        let sides = [(Borders::TOP, rect.y), (Borders::BOTTOM, bottom)];
        for (side, y) in sides {
            if frame.borders.contains(side) {
                for x in rect.x..=right {
                    let cell = cells.entry((x, y)).or_default();
                    cell.left |= x > rect.x;
                    cell.right |= x < right;
                }
            }
        }
        for (side, x) in [(Borders::LEFT, rect.x), (Borders::RIGHT, right)] {
            if frame.borders.contains(side) {
                for y in rect.y..=bottom {
                    let cell = cells.entry((x, y)).or_default();
                    cell.up |= y > rect.y;
                    cell.down |= y < bottom;
                }
            }
        }
    }
    let area = buffer.area;
    for ((x, y), line) in cells {
        if !area.contains((x, y).into()) {
            continue;
        }
        let focused = frames
            .iter()
            .any(|f| f.focused && own_border(x, y, f.rect, f.borders));
        let symbol = symbol(line);
        if symbol.is_empty() {
            continue;
        }
        buffer[(x, y)]
            .set_symbol(symbol)
            .set_style(Style::default().fg(if focused {
                palette.accent
            } else {
                palette.overlay0
            }));
    }
    for frame in frames {
        if !frame.borders.contains(Borders::TOP) || frame.rect.width <= 4 {
            continue;
        }
        let Some(label) = frame
            .label
            .as_deref()
            .map(str::trim)
            .filter(|label| !label.is_empty())
        else {
            continue;
        };
        let title = format!(" {} ", truncate_end(label, frame.rect.width as usize - 4));
        let mut style = Style::default().fg(if frame.focused {
            palette.accent
        } else {
            palette.overlay0
        });
        if frame.focused {
            style = style.add_modifier(Modifier::BOLD);
        }
        let start = frame.rect.x + 1;
        let end = frame.rect.right().saturating_sub(1).min(area.right());
        if start < end && frame.rect.y < area.bottom() {
            buffer.set_stringn(start, frame.rect.y, title, (end - start) as usize, style);
        }
    }
}

/// Herdr's pane scrollbar: a dim track with a brighter thumb, bolder when focused.
pub fn render_scrollbar(
    buffer: &mut Buffer,
    track: Rect,
    metrics: super::chrome::ScrollMetrics,
    palette: &Palette,
    focused: bool,
) {
    let Some(thumb) = super::chrome::scrollbar_thumb(metrics, track) else {
        return;
    };
    let (track_color, thumb_color, thumb_symbol) = if focused {
        (palette.overlay0, palette.overlay1, "▐")
    } else {
        (palette.surface_dim, palette.overlay0, "▕")
    };
    for y in track.y..track.bottom() {
        buffer[(track.x, y)]
            .set_symbol("▕")
            .set_style(Style::default().fg(track_color));
    }
    for y in thumb.top..thumb.top + thumb.len {
        buffer[(track.x, y)]
            .set_symbol(thumb_symbol)
            .set_style(Style::default().fg(thumb_color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_frames_join_and_focus_colors_only_its_edges() {
        let palette = Palette::default();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 4));
        let frames = [
            Frame {
                rect: Rect::new(0, 0, 5, 4),
                borders: Borders::ALL,
                focused: false,
                label: Some("left".into()),
            },
            Frame {
                rect: Rect::new(5, 0, 5, 4),
                borders: Borders::ALL,
                focused: true,
                label: None,
            },
        ];
        render_borders(&mut buffer, &frames, &palette);
        let row = |y: u16| (0..10).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        assert_eq!(row(0), "┌ … ┐┌───┐");
        assert_eq!(row(3), "└───┘└───┘");
        assert_eq!(buffer[(5, 1)].fg, palette.accent);
        assert_eq!(buffer[(4, 1)].fg, palette.overlay0);
    }

    #[test]
    fn chrome_options_follow_herdr() {
        use crate::preferences::PaneBorders;
        let rects = [Rect::new(0, 0, 10, 6), Rect::new(10, 0, 10, 6)];
        let shared = apply_chrome(&rects, PaneBorders::Auto, false, true);
        assert_eq!(shared[0].1, Borders::ALL - Borders::RIGHT);
        assert_eq!(shared[1].1, Borders::ALL);
        let open = apply_chrome(&rects, PaneBorders::Auto, true, false);
        assert_eq!(open[0].1, Borders::RIGHT);
        let gap = apply_chrome(&rects, PaneBorders::Off, true, true);
        assert_eq!(gap[0], (Rect::new(0, 0, 9, 6), Borders::NONE));
        assert_eq!(gap[1], (rects[1], Borders::NONE));
        assert_eq!(
            apply_chrome(&rects[..1], PaneBorders::Auto, true, true)[0].1,
            Borders::NONE
        );
        assert_eq!(
            apply_chrome(&rects[..1], PaneBorders::Always, true, true)[0].1,
            Borders::ALL
        );
    }

    #[test]
    fn scrollbar_gutter_is_reserved_outside_the_alternate_screen() {
        let inner = Rect::new(1, 1, 20, 5);
        assert_eq!(terminal_rect(inner, false).0.width, 19);
        assert_eq!(terminal_rect(inner, true), (inner, None));
    }
}
