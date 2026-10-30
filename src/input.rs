use crate::vt::Vt;
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ghostty_sys::*;
fn mods(m: KeyModifiers) -> GhosttyMods {
    let mut bits = 0;
    for (key, flag) in [
        (KeyModifiers::SHIFT, GHOSTTY_MODS_SHIFT),
        (KeyModifiers::ALT, GHOSTTY_MODS_ALT),
        (KeyModifiers::CONTROL, GHOSTTY_MODS_CTRL),
        (KeyModifiers::SUPER, GHOSTTY_MODS_SUPER),
    ] {
        if m.contains(key) {
            bits |= flag as GhosttyMods;
        }
    }
    bits
}
pub fn key(vt: &mut Vt, event: KeyEvent) -> Vec<u8> {
    if event.kind == KeyEventKind::Release {
        return vec![];
    }
    let text = if let KeyCode::Char(c) = event.code {
        Some(c.to_string())
    } else {
        None
    };
    let key = match event.code {
        KeyCode::Char(c) if c.is_ascii_alphabetic() => {
            GHOSTTY_KEY_A + c.to_ascii_lowercase() as i32 - 'a' as i32
        }
        KeyCode::Char(c) if c.is_ascii_digit() => GHOSTTY_KEY_DIGIT_0 + c as i32 - '0' as i32,
        KeyCode::Char(' ') => GHOSTTY_KEY_SPACE,
        KeyCode::Enter => GHOSTTY_KEY_ENTER,
        KeyCode::Tab | KeyCode::BackTab => GHOSTTY_KEY_TAB,
        KeyCode::Backspace => GHOSTTY_KEY_BACKSPACE,
        KeyCode::Esc => GHOSTTY_KEY_ESCAPE,
        KeyCode::Left => GHOSTTY_KEY_ARROW_LEFT,
        KeyCode::Right => GHOSTTY_KEY_ARROW_RIGHT,
        KeyCode::Up => GHOSTTY_KEY_ARROW_UP,
        KeyCode::Down => GHOSTTY_KEY_ARROW_DOWN,
        KeyCode::Home => GHOSTTY_KEY_HOME,
        KeyCode::End => GHOSTTY_KEY_END,
        KeyCode::PageUp => GHOSTTY_KEY_PAGE_UP,
        KeyCode::PageDown => GHOSTTY_KEY_PAGE_DOWN,
        KeyCode::Insert => GHOSTTY_KEY_INSERT,
        KeyCode::Delete => GHOSTTY_KEY_DELETE,
        KeyCode::F(n) if (1..=25).contains(&n) => GHOSTTY_KEY_F1 + n as i32 - 1,
        _ => GHOSTTY_KEY_UNIDENTIFIED,
    };
    let mut modifiers = mods(event.modifiers);
    if event.code == KeyCode::BackTab {
        modifiers |= GHOSTTY_MODS_SHIFT as u16;
    }
    vt.encode_key(
        key,
        modifiers,
        modifiers & GHOSTTY_MODS_SHIFT as u16,
        text.as_deref(),
        text.as_ref()
            .and_then(|s| s.chars().next())
            .map(|c| c.to_ascii_lowercase()),
    )
}
pub fn mouse(vt: &mut Vt, event: MouseEvent) -> Vec<u8> {
    if !vt.mouse_tracking() && !vt.alt_screen() {
        match event.kind {
            MouseEventKind::ScrollUp => vt.scroll_viewport(-3),
            MouseEventKind::ScrollDown => vt.scroll_viewport(3),
            _ => {}
        }
        return vec![];
    }
    if matches!(
        event.kind,
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
    ) {
        let button = if event.kind == MouseEventKind::ScrollUp {
            GHOSTTY_MOUSE_BUTTON_FOUR
        } else {
            GHOSTTY_MOUSE_BUTTON_FIVE
        };
        return vt.encode_mouse(
            GHOSTTY_MOUSE_ACTION_PRESS,
            Some(button),
            mods(event.modifiers),
            event.column as f32,
            event.row as f32,
            (vt.cols as u32, vt.rows as u32, 1.0, 1.0),
            false,
        );
    }
    let (action, button) = match event.kind {
        MouseEventKind::Down(b) => (GHOSTTY_MOUSE_ACTION_PRESS, Some(b)),
        MouseEventKind::Up(b) => (GHOSTTY_MOUSE_ACTION_RELEASE, Some(b)),
        MouseEventKind::Drag(b) => (GHOSTTY_MOUSE_ACTION_MOTION, Some(b)),
        MouseEventKind::Moved => (GHOSTTY_MOUSE_ACTION_MOTION, None),
        _ => return vec![],
    };
    let button = button.map(|b| match b {
        MouseButton::Left => GHOSTTY_MOUSE_BUTTON_LEFT,
        MouseButton::Right => GHOSTTY_MOUSE_BUTTON_RIGHT,
        MouseButton::Middle => GHOSTTY_MOUSE_BUTTON_MIDDLE,
    });
    vt.encode_mouse(
        action,
        button,
        mods(event.modifiers),
        event.column as f32,
        event.row as f32,
        (vt.cols as u32, vt.rows as u32, 1.0, 1.0),
        matches!(event.kind, MouseEventKind::Drag(_)),
    )
}
