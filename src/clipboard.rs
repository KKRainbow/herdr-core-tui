//! The host clipboard: native helpers with an OSC 52 fallback, and images for paste.
//! Adapted from Herdr `src/platform/linux/clipboard.rs`, `src/platform/linux.rs`,
//! `src/selection.rs`, and `src/client/clipboard_images.rs` at
//! ee5bfb5041bb3e4a49d591c72308aae3c81ecdde.
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};
#[cfg(unix)]
use {
    std::io::Read,
    std::os::fd::{AsRawFd, RawFd},
    std::os::unix::process::CommandExt,
    std::process::{Child, Stdio},
};

// Allow slow desktop startup, but never let a clipboard operation wedge the client.
const TIMEOUT: Duration = Duration::from_secs(5);
/// Core's `terminal.put` limit.
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

#[cfg(unix)]
struct Helper {
    child: Child,
    completed: bool,
}

#[cfg(unix)]
impl Drop for Helper {
    fn drop(&mut self) {
        if !self.completed {
            // Wrappers may leave descendants holding the pipes after their parent exits.
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(unix)]
fn nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn run(
    mut command: Command,
    input: Option<&[u8]>,
    max_bytes: usize,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    if Instant::now() >= deadline {
        return Err(io::ErrorKind::TimedOut.into());
    }
    let child = command
        .process_group(0)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if max_bytes > 0 {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::null())
        .spawn()?;
    let mut helper = Helper {
        child,
        completed: false,
    };
    let mut stdin = helper.child.stdin.take();
    let mut stdout = helper.child.stdout.take();
    if let Some(pipe) = &stdin {
        nonblocking(pipe.as_raw_fd())?;
    }
    if let Some(pipe) = &stdout {
        nonblocking(pipe.as_raw_fd())?;
    }
    let input = input.unwrap_or_default();
    let mut written = 0;
    let mut bytes = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let mut progress = false;
        if written == input.len() {
            stdin = None;
        }
        if let Some(pipe) = stdin.as_mut() {
            match pipe.write(&input[written..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(count) => {
                    written += count;
                    progress = true;
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(err) => return Err(err),
            }
        }
        if let Some(pipe) = stdout.as_mut() {
            let mut buffer = [0; 8192];
            let limit = buffer
                .len()
                .min(max_bytes.saturating_sub(bytes.len()).saturating_add(1));
            match pipe.read(&mut buffer[..limit]) {
                Ok(0) => stdout = None,
                Ok(count) => {
                    if count > max_bytes - bytes.len() {
                        return Err(io::ErrorKind::FileTooLarge.into());
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                    progress = true;
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(err) => return Err(err),
            }
        }
        if let Some(status) = helper.child.try_wait()? {
            if !status.success() {
                return Err(io::Error::other("clipboard helper failed"));
            }
            if written == input.len() && stdout.is_none() {
                helper.completed = true;
                return Ok(bytes);
            }
        }
        if !progress {
            std::thread::sleep(
                Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
}

/// Windows helpers: a reader thread and a deadline; no process groups to reap.
#[cfg(windows)]
fn run(
    mut command: Command,
    input: Option<&[u8]>,
    max_bytes: usize,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    if let (Some(mut pipe), Some(input)) = (child.stdin.take(), input) {
        pipe.write_all(input)?;
    }
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = (&mut stdout)
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes);
        read.map(|_| bytes)
    });
    loop {
        if let Some(status) = child.try_wait()? {
            let bytes = reader
                .join()
                .map_err(|_| io::Error::other("reader failed"))??;
            if !status.success() {
                return Err(io::Error::other("clipboard helper failed"));
            }
            if bytes.len() > max_bytes {
                return Err(io::ErrorKind::FileTooLarge.into());
            }
            return Ok(bytes);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::ErrorKind::TimedOut.into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Herdr's writers: `wl-copy` on Wayland, `xclip` or `xsel` on X11, `pbcopy` on macOS.
fn write_commands() -> Vec<(&'static str, &'static [&'static str])> {
    let mut commands: Vec<(&'static str, &'static [&'static str])> = Vec::new();
    if cfg!(target_os = "macos") {
        commands.push(("pbcopy", &[]));
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        commands.push(("wl-copy", &["--type", "text/plain;charset=utf-8"]));
    }
    if std::env::var_os("DISPLAY").is_some() {
        commands.push(("xclip", &["-selection", "clipboard", "-in"]));
        commands.push(("xsel", &["--clipboard", "--input"]));
    }
    commands
}

/// Over SSH, in VS Code, or in WSL the local helpers reach the wrong clipboard.
fn prefer_osc52() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some()
        || std::env::var_os("SSH_TTY").is_some()
        || std::env::var_os("VSCODE_IPC_HOOK_CLI").is_some()
        || std::env::var_os("WSL_DISTRO_NAME").is_some()
        || std::env::var_os("WSL_INTEROP").is_some()
}

/// Copies text as Herdr does: a native helper when one works here, else OSC 52.
pub fn copy(text: &str) -> io::Result<()> {
    if !prefer_osc52() {
        let deadline = Instant::now() + TIMEOUT;
        for (program, args) in write_commands() {
            let mut command = Command::new(program);
            command.args(args);
            if run(command, Some(text.as_bytes()), 0, deadline).is_ok() {
                return Ok(());
            }
        }
    }
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut stdout = io::stdout();
    write!(stdout, "\x1b]52;c;{}\x07", STANDARD.encode(text))?;
    stdout.flush()
}

/// An image from the clipboard or a dropped file: its bytes and file extension.
pub struct Image {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
}

fn matches_signature(extension: &str, bytes: &[u8]) -> bool {
    match extension {
        "png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "jpg" => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "webp" => bytes.len() >= 12 && bytes.starts_with(b"RIFF") && bytes[8..12] == *b"WEBP",
        "bmp" => {
            if bytes.len() < 26 || !bytes.starts_with(b"BM") {
                return false;
            }
            let offset = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
            (26..=bytes.len()).contains(&offset)
        }
        _ => false,
    }
}

/// The Windows clipboard's image through PowerShell, as Herdr reads it under WSL.
#[cfg(windows)]
pub fn read_image() -> Option<Image> {
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-STA",
        "-Command",
        "$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $image=[System.Windows.Forms.Clipboard]::GetImage(); if ($null -eq $image) { exit 1 }; $stream=[System.IO.MemoryStream]::new(); try { $image.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png); [Console]::OpenStandardOutput().Write($stream.GetBuffer(), 0, [int]$stream.Length) } finally { $stream.Dispose(); $image.Dispose() }",
    ]);
    let bytes = run(command, None, MAX_IMAGE_BYTES, Instant::now() + TIMEOUT).ok()?;
    matches_signature("png", &bytes).then_some(Image {
        bytes,
        extension: "png",
    })
}

/// The clipboard's image, tried in Herdr's order of types and tools.
#[cfg(not(windows))]
pub fn read_image() -> Option<Image> {
    let deadline = Instant::now() + TIMEOUT;
    for (mime, extension) in [
        ("image/png", "png"),
        ("image/jpeg", "jpg"),
        ("image/jpg", "jpg"),
        ("image/gif", "gif"),
        ("image/webp", "webp"),
        ("image/bmp", "bmp"),
    ] {
        let mut commands = Vec::new();
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            commands.push(("wl-paste", vec!["--type", mime]));
        }
        if std::env::var_os("DISPLAY").is_some() {
            commands.push(("xclip", vec!["-selection", "clipboard", "-t", mime, "-o"]));
        }
        for (program, args) in commands {
            let mut command = Command::new(program);
            command.args(args);
            if let Ok(bytes) = run(command, None, MAX_IMAGE_BYTES, deadline)
                && !bytes.is_empty()
                && matches_signature(extension, &bytes)
            {
                return Some(Image { bytes, extension });
            }
        }
    }
    None
}

/// A dropped image file pasted as its absolute path, as Herdr bridges it.
pub fn dropped_image(text: &str) -> Option<Image> {
    let text = text.trim_end_matches(['\r', '\n']);
    if text.is_empty() || text.contains(['\r', '\n']) {
        return None;
    }
    let text = unescape_drop_path(strip_matching_path_quotes(text));
    let path = PathBuf::from(text);
    if !path.is_absolute() {
        return None;
    }
    let extension = recognized_image_extension(path.extension()?.to_str()?)?;
    let metadata = std::fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() as usize > MAX_IMAGE_BYTES {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    (!bytes.is_empty()).then_some(Image { bytes, extension })
}

fn strip_matching_path_quotes(text: &str) -> &str {
    if text.len() < 2 {
        return text;
    }
    let bytes = text.as_bytes();
    match (bytes.first(), bytes.last()) {
        (Some(b'\''), Some(b'\'')) | (Some(b'"'), Some(b'"')) => &text[1..text.len() - 1],
        _ => text,
    }
}

fn unescape_drop_path(text: &str) -> String {
    let mut unescaped = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            unescaped.push(chars.next().unwrap_or(ch));
        } else {
            unescaped.push(ch);
        }
    }
    unescaped
}

fn recognized_image_extension(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpg"),
        "gif" => Some("gif"),
        "webp" => Some("webp"),
        "bmp" => Some("bmp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_reject_text_as_images() {
        assert!(matches_signature("png", b"\x89PNG\r\n\x1a\nrest"));
        assert!(!matches_signature("png", b"not an image"));
        assert!(matches_signature("jpg", &[0xFF, 0xD8, 0xFF, 0]));
    }

    #[test]
    fn dropped_paths_unquote_and_need_an_image_extension() {
        let dir = std::env::temp_dir().join(format!("tui-drop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shot one.png");
        std::fs::write(&file, b"\x89PNG\r\n\x1a\n").unwrap();
        let escaped = file.display().to_string().replace(' ', "\\ ");
        assert_eq!(dropped_image(&escaped).unwrap().extension, "png");
        assert!(dropped_image(&format!("'{}'", file.display())).is_some());
        assert!(dropped_image("relative.png").is_none());
        assert!(dropped_image(&dir.join("notes.txt").display().to_string()).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
