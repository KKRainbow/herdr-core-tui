use crate::{copy::Copy, vt};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
};
pub fn terminal(
    frame: &mut Frame,
    area: Rect,
    snapshot: &vt::Frame,
    focused: bool,
    selection: Option<&Copy>,
) {
    let buffer = frame.buffer_mut();
    for (y, row) in snapshot.rows.iter().take(area.height as usize).enumerate() {
        let mut tail = false;
        for (x, cell) in row.iter().take(area.width as usize).enumerate() {
            if tail {
                tail = false;
                continue;
            }
            let mut style = Style::default()
                .fg(color(cell.fg))
                .bg(color(cell.bg.unwrap_or(snapshot.bg)));
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
                style = style.add_modifier(Modifier::REVERSED);
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
    if focused
        && let Some((x, y)) = selection
            .map(|s| s.cursor)
            .or(snapshot.cursor)
            .filter(|(x, y)| *x < area.width && *y < area.height)
    {
        frame.set_cursor_position((area.x + x, area.y + y));
    }
}
fn color(c: vt::Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}
