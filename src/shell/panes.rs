//! Pane frames: Herdr draws borders only when a tab shows several panes, joins the
//! lines into box-drawing junctions, and highlights the focused pane's edges.
//! Adapted from Herdr `src/ui/panes.rs` with its default `pane_borders = "auto"`,
//! `pane_outer_borders = true`, and `pane_gaps = true`.
use super::{chrome::truncate_end, palette::Palette};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub rect: Rect,
    pub bordered: bool,
    pub focused: bool,
    pub label: Option<String>,
}

/// The cells inside a pane's frame.
pub fn inner(rect: Rect, bordered: bool) -> Rect {
    if !bordered {
        return rect;
    }
    Rect::new(
        rect.x.saturating_add(1),
        rect.y.saturating_add(1),
        rect.width.saturating_sub(2),
        rect.height.saturating_sub(2),
    )
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

fn own_border(x: u16, y: u16, rect: Rect) -> bool {
    if rect.is_empty() {
        return false;
    }
    let right = rect.right() - 1;
    let bottom = rect.bottom() - 1;
    let in_rows = y >= rect.y && y <= bottom;
    let in_cols = x >= rect.x && x <= right;
    (in_rows && (x == rect.x || x == right)) || (in_cols && (y == rect.y || y == bottom))
}

pub fn render_borders(buffer: &mut Buffer, frames: &[Frame], palette: &Palette) {
    let mut cells = HashMap::<(u16, u16), LineCell>::new();
    for frame in frames.iter().filter(|f| f.bordered && !f.rect.is_empty()) {
        let rect = frame.rect;
        let right = rect.right() - 1;
        let bottom = rect.bottom() - 1;
        for x in rect.x..=right {
            for y in [rect.y, bottom] {
                let cell = cells.entry((x, y)).or_default();
                cell.left |= x > rect.x;
                cell.right |= x < right;
            }
        }
        for y in rect.y..=bottom {
            for x in [rect.x, right] {
                let cell = cells.entry((x, y)).or_default();
                cell.up |= y > rect.y;
                cell.down |= y < bottom;
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
            .any(|f| f.bordered && f.focused && own_border(x, y, f.rect));
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
        if !frame.bordered || frame.rect.width <= 4 {
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
                bordered: true,
                focused: false,
                label: Some("left".into()),
            },
            Frame {
                rect: Rect::new(5, 0, 5, 4),
                bordered: true,
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
    fn scrollbar_gutter_is_reserved_outside_the_alternate_screen() {
        let inner = Rect::new(1, 1, 20, 5);
        assert_eq!(terminal_rect(inner, false).0.width, 19);
        assert_eq!(terminal_rect(inner, true), (inner, None));
    }
}
