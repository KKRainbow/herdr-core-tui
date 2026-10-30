use crate::{copy::Copy, shell::palette::Palette, vt};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
};
pub fn terminal(
    frame: &mut Frame,
    area: Rect,
    snapshot: &vt::Frame,
    selection: Option<&Copy>,
    palette: &Palette,
) {
    let selected = selection_style(palette);
    let buffer = frame.buffer_mut();
    for (y, row) in snapshot.rows.iter().take(area.height as usize).enumerate() {
        let mut tail = false;
        for (x, cell) in row.iter().take(area.width as usize).enumerate() {
            if tail {
                tail = false;
                continue;
            }
            let mut style = Style::default()
                .fg(if cell.default_fg {
                    Color::Reset
                } else {
                    color(cell.fg)
                })
                .bg(cell.bg.map_or(Color::Reset, color));
            for (enabled, modifier) in [
                (cell.bold, Modifier::BOLD),
                (cell.faint, Modifier::DIM),
                (cell.italic, Modifier::ITALIC),
                (cell.underline, Modifier::UNDERLINED),
                (cell.strike, Modifier::CROSSED_OUT),
            ] {
                if enabled {
                    style = style.add_modifier(modifier);
                }
            }
            if selection.is_some_and(|s| s.contains(x as u16, y as u16)) {
                style = selected;
            }
            buffer[(area.x + x as u16, area.y + y as u16)]
                .set_symbol(if cell.text.is_empty() {
                    " "
                } else {
                    &cell.text
                })
                .set_style(style);
            tail = cell.wide;
        }
    }
}
fn color(c: vt::Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

/// Herdr's selection colors: the panel background lifted toward white (or black on light themes).
fn selection_style(palette: &Palette) -> Style {
    let base = match palette.panel_bg {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => return Style::reset().add_modifier(Modifier::REVERSED),
    };
    let luminance = |(r, g, b): (u8, u8, u8)| {
        let channel = |c: u8| {
            let c = f32::from(c) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    };
    let target = if luminance(base) < 0.5 { 255.0 } else { 0.0 };
    let mix = |c: u8| (f32::from(c) + (target - f32::from(c)) * 0.28).round() as u8;
    let bg = (mix(base.0), mix(base.1), mix(base.2));
    let fg = if (luminance(bg) + 0.05) / 0.05 > 1.05 / (luminance(bg) + 0.05) {
        Color::Rgb(0, 0, 0)
    } else {
        Color::Rgb(255, 255, 255)
    };
    Style::reset().fg(fg).bg(Color::Rgb(bg.0, bg.1, bg.2))
}
