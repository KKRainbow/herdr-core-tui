//! Agent notifications outside the TUI, adapted from Herdr `src/terminal_notify.rs`
//! and the `show_desktop_notification` helpers in `src/platform/{linux,macos}.rs`
//! at ee5bfb5041bb3e4a49d591c72308aae3c81ecdde.
use std::{
    io::{self, Write as _},
    process::{Command, Stdio},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalNotificationBackend {
    Ghostty,
    Iterm2,
    Kitty,
    WezTerm,
}

pub fn detect_backend() -> Option<TerminalNotificationBackend> {
    let term_program = std::env::var("TERM_PROGRAM").ok();
    let term = std::env::var("TERM").ok();
    match term_program.as_deref() {
        Some("ghostty") => return Some(TerminalNotificationBackend::Ghostty),
        Some("iTerm.app") => return Some(TerminalNotificationBackend::Iterm2),
        Some("WezTerm") => return Some(TerminalNotificationBackend::WezTerm),
        _ => {}
    }
    if std::env::var_os("KITTY_WINDOW_ID").is_some() {
        return Some(TerminalNotificationBackend::Kitty);
    }
    match term.as_deref() {
        Some("xterm-ghostty") => Some(TerminalNotificationBackend::Ghostty),
        Some("xterm-kitty") => Some(TerminalNotificationBackend::Kitty),
        Some(term) if term.contains("wezterm") => Some(TerminalNotificationBackend::WezTerm),
        _ => None,
    }
}

/// Asks the host terminal to show a notification; false when it has no known way.
pub fn show_terminal_notification(title: &str, body: Option<&str>) -> io::Result<bool> {
    let Some(backend) = detect_backend() else {
        return Ok(false);
    };
    let sequence = match backend {
        TerminalNotificationBackend::Ghostty
        | TerminalNotificationBackend::Iterm2
        | TerminalNotificationBackend::WezTerm => build_osc9_notification(title, body),
        TerminalNotificationBackend::Kitty => build_osc99_notification(title, body),
    };
    let sequence = if std::env::var_os("TMUX").is_some() {
        wrap_tmux_passthrough(&sequence)
    } else {
        sequence
    };
    let mut stdout = io::stdout();
    stdout.write_all(&sequence)?;
    stdout.flush()?;
    Ok(true)
}

fn build_osc9_notification(title: &str, body: Option<&str>) -> Vec<u8> {
    let message = sanitize_text(match body {
        Some(body) if !body.is_empty() => format!("{title}: {body}"),
        _ => title.to_string(),
    });
    format!("\x1b]9;{message}\x1b\\").into_bytes()
}

fn build_osc99_notification(title: &str, body: Option<&str>) -> Vec<u8> {
    let title = sanitize_text(title);
    match body {
        Some(body) if !body.is_empty() => {
            let body = sanitize_text(body);
            format!("\x1b]99;i=1:d=0;{title}\x1b\\\x1b]99;i=1:p=body;{body}\x1b\\").into_bytes()
        }
        _ => format!("\x1b]99;;{title}\x1b\\").into_bytes(),
    }
}

fn sanitize_text(text: impl AsRef<str>) -> String {
    text.as_ref()
        .chars()
        .filter(|ch| *ch != '\u{1b}' && *ch != '\u{7}' && *ch != '\u{9c}')
        .map(|ch| match ch {
            '\n' | '\r' | '\t' => ' ',
            _ => ch,
        })
        .collect()
}

fn wrap_tmux_passthrough(sequence: &[u8]) -> Vec<u8> {
    let mut wrapped = Vec::with_capacity(sequence.len() + 16);
    wrapped.extend_from_slice(b"\x1bPtmux;");
    for &byte in sequence {
        if byte == 0x1b {
            wrapped.push(0x1b);
        }
        wrapped.push(byte);
    }
    wrapped.extend_from_slice(b"\x1b\\");
    wrapped
}

/// The desktop's own notification: `notify-send`, or `osascript` on macOS.
pub fn show_desktop_notification(title: &str, body: Option<&str>) -> io::Result<bool> {
    let mut command = if cfg!(target_os = "macos") {
        let mut cmd = Command::new("/usr/bin/osascript");
        cmd.arg("-e")
            .arg("on run argv")
            .arg("-e")
            .arg("display notification (item 2 of argv) with title (item 1 of argv)")
            .arg("-e")
            .arg("end run")
            .arg(title)
            .arg(body.unwrap_or_default());
        cmd
    } else {
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Ok(false);
        }
        let program = std::env::var_os("HERDR_TUI_NOTIFY_SEND").unwrap_or("notify-send".into());
        let mut cmd = Command::new(program);
        cmd.arg("--app-name").arg("herdr-tui").arg("--").arg(title);
        if let Some(body) = body.filter(|body| !body.is_empty()) {
            cmd.arg(body);
        }
        cmd
    };
    let status = match command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) => status,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    Ok(status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_text_strips_control_bytes() {
        assert_eq!(sanitize_text("a\n\tb\u{1b}c\u{7}"), "a  bc");
    }

    #[test]
    fn kitty_notification_uses_structured_title_and_body() {
        let sequence =
            String::from_utf8(build_osc99_notification("pi finished", Some("ws · 1"))).unwrap();
        assert!(sequence.contains("]99;i=1:d=0;pi finished"));
        assert!(sequence.contains("]99;i=1:p=body;ws · 1"));
    }

    #[test]
    fn tmux_passthrough_wraps_and_escapes() {
        let wrapped = wrap_tmux_passthrough(b"\x1b]9;hi\x1b\\");
        assert_eq!(wrapped, b"\x1bPtmux;\x1b\x1b]9;hi\x1b\x1b\\\x1b\\");
    }
}
