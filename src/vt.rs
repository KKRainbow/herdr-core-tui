//! Terminal parsing, cell extraction, and input encoding adapted from herdr-app.
use ghostty_sys::*;
use std::ffi::c_void;
const BRACKETED_PASTE: GhosttyMode = 2004;
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl From<GhosttyColorRgb> for Rgb {
    fn from(c: GhosttyColorRgb) -> Self {
        Rgb(c.r, c.g, c.b)
    }
}

#[derive(Clone, Default, PartialEq)]
pub struct Cell {
    /// Empty for blank cells and for the tail half of a wide character.
    pub text: String,
    pub fg: Rgb,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub underline: bool,
    pub strike: bool,
    pub wide: bool,
}

pub struct Frame {
    pub rows: Vec<Vec<Cell>>,
    pub cursor: Option<(u16, u16)>,
    pub bg: Rgb,
}
pub struct Vt {
    term: GhosttyTerminal,
    state: GhosttyRenderState,
    rows_it: GhosttyRenderStateRowIterator,
    cells: GhosttyRenderStateRowCells,
    keys: GhosttyKeyEncoder,
    key_event: GhosttyKeyEvent,
    mouse: GhosttyMouseEncoder,
    mouse_event: GhosttyMouseEvent,
    pub cols: u16,
    pub rows: u16,
}
fn ok(result: GhosttyResult, operation: &str) {
    assert_eq!(result, GHOSTTY_SUCCESS, "{operation}");
}
impl Vt {
    pub fn new(cols: u16, rows: u16) -> Self {
        unsafe {
            let mut vt = Self {
                term: std::ptr::null_mut(),
                state: std::ptr::null_mut(),
                rows_it: std::ptr::null_mut(),
                cells: std::ptr::null_mut(),
                keys: std::ptr::null_mut(),
                key_event: std::ptr::null_mut(),
                mouse: std::ptr::null_mut(),
                mouse_event: std::ptr::null_mut(),
                cols: cols.max(1),
                rows: rows.max(1),
            };
            ok(
                ghostty_terminal_new(std::ptr::null(), &mut vt.term, vt.cols, vt.rows),
                "terminal",
            );
            ok(
                ghostty_render_state_new(std::ptr::null(), &mut vt.state),
                "render state",
            );
            ok(
                ghostty_render_state_row_iterator_new(std::ptr::null(), &mut vt.rows_it),
                "rows",
            );
            ok(
                ghostty_render_state_row_cells_new(std::ptr::null(), &mut vt.cells),
                "cells",
            );
            ok(
                ghostty_key_encoder_new(std::ptr::null(), &mut vt.keys),
                "keys",
            );
            ok(
                ghostty_key_event_new(std::ptr::null(), &mut vt.key_event),
                "key event",
            );
            ok(
                ghostty_mouse_encoder_new(std::ptr::null(), &mut vt.mouse),
                "mouse",
            );
            ok(
                ghostty_mouse_event_new(std::ptr::null(), &mut vt.mouse_event),
                "mouse event",
            );
            let limit: usize = 10_000_000;
            ok(
                ghostty_terminal_set(
                    vt.term,
                    GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES,
                    &limit as *const _ as *const c_void,
                ),
                "scrollback limit",
            );
            vt
        }
    }
    pub fn write(&mut self, bytes: &[u8]) {
        unsafe { ghostty_terminal_vt_write(self.term, bytes.as_ptr(), bytes.len()) };
    }
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols.max(1);
        self.rows = rows.max(1);
        unsafe { ghostty_terminal_resize(self.term, self.cols, self.rows, 1, 1) };
    }
    pub fn frame(&mut self) -> Frame {
        unsafe {
            ghostty_render_state_update(self.state, self.term);
            let mut colors = GhosttyRenderStateColors {
                size: std::mem::size_of::<GhosttyRenderStateColors>(),
                ..Default::default()
            };
            ghostty_render_state_get(
                self.state,
                GHOSTTY_RENDER_STATE_DATA_COLORS,
                &mut colors as *mut _ as *mut c_void,
            );
            ghostty_render_state_get(
                self.state,
                GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                &mut self.rows_it as *mut _ as *mut c_void,
            );
            let mut rows = Vec::with_capacity(self.rows as usize);
            while ghostty_render_state_row_iterator_next(self.rows_it) {
                ghostty_render_state_row_get(
                    self.rows_it,
                    GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                    &mut self.cells as *mut _ as *mut c_void,
                );
                let mut row = Vec::with_capacity(self.cols as usize);
                while ghostty_render_state_row_cells_next(self.cells) {
                    row.push(self.read_cell(colors.foreground.into(), colors.background.into()));
                }
                rows.push(row);
            }
            let mut cursor = GhosttyRenderStateCursor {
                size: std::mem::size_of::<GhosttyRenderStateCursor>(),
                ..Default::default()
            };
            ghostty_render_state_get(
                self.state,
                GHOSTTY_RENDER_STATE_DATA_CURSOR,
                &mut cursor as *mut _ as *mut c_void,
            );
            ghostty_render_state_clean(self.state);
            Frame {
                rows,
                cursor: (cursor.visible && cursor.viewport_has_value)
                    .then_some((cursor.viewport_x, cursor.viewport_y)),
                bg: colors.background.into(),
            }
        }
    }
    unsafe fn read_cell(&mut self, fg_default: Rgb, bg_default: Rgb) -> Cell {
        unsafe {
            let c = self.cells;
            let mut raw: GhosttyCell = Default::default();
            ghostty_render_state_row_cells_get(
                c,
                GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW,
                &mut raw as *mut _ as *mut c_void,
            );
            let mut wide: GhosttyCellWide = GHOSTTY_CELL_WIDE_NARROW;
            ghostty_cell_get(
                raw,
                GHOSTTY_CELL_DATA_WIDE,
                &mut wide as *mut _ as *mut c_void,
            );

            let mut style = GhosttyStyle {
                size: std::mem::size_of::<GhosttyStyle>(),
                ..Default::default()
            };
            ghostty_render_state_row_cells_get(
                c,
                GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
                &mut style as *mut _ as *mut c_void,
            );

            let mut fg = GhosttyColorRgb::default();
            let fg = if ghostty_render_state_row_cells_get(
                c,
                GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR,
                &mut fg as *mut _ as *mut c_void,
            ) == GHOSTTY_SUCCESS
            {
                Rgb::from(fg)
            } else {
                fg_default
            };
            let mut bg = GhosttyColorRgb::default();
            let bg = (ghostty_render_state_row_cells_get(
                c,
                GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR,
                &mut bg as *mut _ as *mut c_void,
            ) == GHOSTTY_SUCCESS)
                .then(|| Rgb::from(bg));

            let (mut fg, mut bg) = (fg, bg);
            if style.inverse {
                let new_fg = bg.unwrap_or(bg_default);
                bg = Some(fg);
                fg = new_fg;
            }
            let mut text = String::new();
            if wide != GHOSTTY_CELL_WIDE_SPACER_TAIL
                && wide != GHOSTTY_CELL_WIDE_SPACER_HEAD
                && !style.invisible
            {
                let mut len: u32 = 0;
                ghostty_render_state_row_cells_get(
                    c,
                    GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN,
                    &mut len as *mut _ as *mut c_void,
                );
                if len > 0 {
                    let mut cps = vec![0u32; len as usize];
                    ghostty_render_state_row_cells_get(
                        c,
                        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF,
                        cps.as_mut_ptr() as *mut c_void,
                    );
                    text = cps.into_iter().filter_map(char::from_u32).collect();
                }
            }

            Cell {
                text,
                fg,
                bg,
                bold: style.bold,
                italic: style.italic,
                faint: style.faint,
                underline: style.underline != 0,
                strike: style.strikethrough,
                wide: wide == GHOSTTY_CELL_WIDE_WIDE,
            }
        }
    }

    #[cfg(test)]
    pub fn encode_key(
        &mut self,
        key: GhosttyKey,
        mods: GhosttyMods,
        consumed: GhosttyMods,
        text: Option<&str>,
        unshifted: Option<char>,
    ) -> Vec<u8> {
        self.encode_key_action(
            key,
            mods,
            consumed,
            text,
            unshifted,
            GHOSTTY_KEY_ACTION_PRESS,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode_key_action(
        &mut self,
        key: GhosttyKey,
        mods: GhosttyMods,
        consumed: GhosttyMods,
        text: Option<&str>,
        unshifted: Option<char>,
        action: GhosttyKeyAction,
    ) -> Vec<u8> {
        unsafe {
            ghostty_key_encoder_setopt_from_terminal(self.keys, self.term);
            let e = self.key_event;
            ghostty_key_event_set_action(e, action);
            ghostty_key_event_set_key(e, key);
            ghostty_key_event_set_mods(e, mods);
            let printable =
                text.is_some_and(|t| !t.is_empty() && !t.chars().any(|c| c.is_control()));
            ghostty_key_event_set_consumed_mods(e, if printable { consumed } else { 0 });
            match text {
                Some(t) if printable => {
                    ghostty_key_event_set_utf8(e, t.as_ptr() as *const _, t.len())
                }
                _ => ghostty_key_event_set_utf8(e, std::ptr::null(), 0),
            }
            ghostty_key_event_set_unshifted_codepoint(e, unshifted.map_or(0, |c| c as u32));
            let mut buf = [0u8; 128];
            let mut len = 0usize;
            let r = ghostty_key_encoder_encode(
                self.keys,
                e,
                buf.as_mut_ptr() as *mut _,
                buf.len(),
                &mut len,
            );
            if r != GHOSTTY_SUCCESS {
                return Vec::new();
            }
            buf[..len].to_vec()
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode_mouse(
        &mut self,
        action: GhosttyMouseAction,
        button: Option<GhosttyMouseButton>,
        mods: GhosttyMods,
        x: f32,
        y: f32,
        // (columns, rows, cell width, cell height), cells in px.
        size: (u32, u32, f32, f32),
        any_pressed: bool,
    ) -> Vec<u8> {
        // Ghostty takes whole-pixel cells; at zoom a cell is 11.7 px, and
        // rounding it alone made the reported column run ahead of the pointer
        // (2 cells by mid-line). So the position moves into a grid of whole
        // cells: same column and row exactly, pixels (SGR-pixels mode) within
        // a few percent.
        let (cols, rows, cw, ch) = size;
        let (cwi, chi) = (cw.round().max(1.0), ch.round().max(1.0));
        let (x, y) = (x / cw.max(1.0) * cwi, y / ch.max(1.0) * chi);
        unsafe {
            ghostty_mouse_encoder_setopt_from_terminal(self.mouse, self.term);
            let s = GhosttyMouseEncoderSize {
                size: std::mem::size_of::<GhosttyMouseEncoderSize>(),
                screen_width: cols * cwi as u32,
                screen_height: rows * chi as u32,
                cell_width: cwi as u32,
                cell_height: chi as u32,
                ..Default::default()
            };
            ghostty_mouse_encoder_setopt(
                self.mouse,
                GHOSTTY_MOUSE_ENCODER_OPT_SIZE,
                &s as *const _ as *const c_void,
            );
            ghostty_mouse_encoder_setopt(
                self.mouse,
                GHOSTTY_MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED,
                &any_pressed as *const _ as *const c_void,
            );
            let e = self.mouse_event;
            ghostty_mouse_event_set_action(e, action);
            match button {
                Some(b) => ghostty_mouse_event_set_button(e, b),
                None => ghostty_mouse_event_clear_button(e),
            }
            ghostty_mouse_event_set_mods(e, mods);
            ghostty_mouse_event_set_position(e, GhosttyMousePosition { x, y });
            let mut buf = [0u8; 64];
            let mut len = 0usize;
            let r = ghostty_mouse_encoder_encode(
                self.mouse,
                e,
                buf.as_mut_ptr() as *mut _,
                buf.len(),
                &mut len,
            );
            if r != GHOSTTY_SUCCESS {
                return Vec::new();
            }
            buf[..len].to_vec()
        }
    }
    fn get_bool(&self, data: GhosttyTerminalData) -> bool {
        let mut v = false;
        unsafe { ghostty_terminal_get(self.term, data, &mut v as *mut _ as *mut c_void) };
        v
    }

    fn mode(&self, mode: GhosttyMode) -> bool {
        let mut cfg = GhosttyTerminalModeConfig { mode, value: false };
        unsafe {
            ghostty_terminal_get(
                self.term,
                GHOSTTY_TERMINAL_DATA_MODE,
                &mut cfg as *mut _ as *mut c_void,
            )
        };
        cfg.value
    }

    pub fn mouse_tracking(&self) -> bool {
        self.get_bool(GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING)
    }

    pub fn alt_screen(&self) -> bool {
        let mut screen: GhosttyTerminalScreen = GHOSTTY_TERMINAL_SCREEN_PRIMARY;
        unsafe {
            ghostty_terminal_get(
                self.term,
                GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN,
                &mut screen as *mut _ as *mut c_void,
            )
        };
        screen == GHOSTTY_TERMINAL_SCREEN_ALTERNATE
    }

    pub fn scroll_viewport(&mut self, delta: isize) {
        let mut s = GhosttyTerminalScrollViewport {
            tag: GHOSTTY_SCROLL_VIEWPORT_DELTA,
            ..Default::default()
        };
        s.value.delta = delta;
        unsafe { ghostty_terminal_scroll_viewport(self.term, s) };
    }

    pub fn scroll_to_bottom(&mut self) {
        let s = GhosttyTerminalScrollViewport {
            tag: GHOSTTY_SCROLL_VIEWPORT_BOTTOM,
            ..Default::default()
        };
        unsafe { ghostty_terminal_scroll_viewport(self.term, s) };
    }

    /// Wraps pasted text the way the program asked (bracketed paste or not).
    pub fn paste_bytes(&self, text: &str) -> Vec<u8> {
        if self.mode(BRACKETED_PASTE) {
            let clean = text.replace("\x1b[201~", "");
            [b"\x1b[200~".as_slice(), clean.as_bytes(), b"\x1b[201~"].concat()
        } else {
            text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
        }
    }
}
impl Drop for Vt {
    fn drop(&mut self) {
        unsafe {
            ghostty_mouse_event_free(self.mouse_event);
            ghostty_mouse_encoder_free(self.mouse);
            ghostty_key_event_free(self.key_event);
            ghostty_key_encoder_free(self.keys);
            ghostty_render_state_row_cells_free(self.cells);
            ghostty_render_state_row_iterator_free(self.rows_it);
            ghostty_render_state_free(self.state);
            ghostty_terminal_free(self.term);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_colors_cursor_and_alternate_screen() {
        let mut vt = Vt::new(20, 4);
        vt.write("\x1b[38;2;1;2;3m界e\u{301}".as_bytes());
        let frame = vt.frame();
        assert_eq!(frame.rows[0][0].text, "界");
        assert!(frame.rows[0][0].wide);
        assert_eq!(frame.rows[0][2].text, "e\u{301}");
        assert_eq!(frame.rows[0][0].fg, Rgb(1, 2, 3));
        assert_eq!(frame.cursor, Some((3, 0)));
        vt.write(b"\x1b[?1049h\x1b[HALT");
        assert!(vt.alt_screen());
        assert_eq!(vt.frame().rows[0][0].text, "A");
        vt.write(b"\x1b[?1049l");
        assert!(!vt.alt_screen());
        assert_eq!(vt.frame().rows[0][0].text, "界");
        vt.resize(30, 8);
        assert_eq!(vt.frame().rows.len(), 8);
    }
    #[test]
    fn input_follows_terminal_modes() {
        let mut vt = Vt::new(20, 4);
        assert_eq!(
            vt.encode_key(GHOSTTY_KEY_ARROW_UP, 0, 0, None, None),
            b"\x1b[A"
        );
        vt.write(b"\x1b[?1h\x1b[?2004h\x1b[?1000h\x1b[?1006h");
        assert_eq!(
            vt.encode_key(GHOSTTY_KEY_ARROW_UP, 0, 0, None, None),
            b"\x1bOA"
        );
        assert_eq!(vt.paste_bytes("hello"), b"\x1b[200~hello\x1b[201~");
        assert!(vt.mouse_tracking());
        assert_eq!(
            vt.encode_mouse(
                GHOSTTY_MOUSE_ACTION_PRESS,
                Some(GHOSTTY_MOUSE_BUTTON_LEFT),
                0,
                2.,
                1.,
                (20, 4, 1., 1.),
                false
            ),
            b"\x1b[<0;3;2M"
        );
    }
}
