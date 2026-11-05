//! Agent notification sounds, adapted from Herdr `src/sound.rs`.
//! The mp3 files are Herdr's own; a system player decodes them, so no audio crate is linked.
use herdr_client_lib::model::Alert;
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

const DISABLE_ENV: &str = "HERDR_TUI_DISABLE_SOUND";
const PLAYER_TIMEOUT: Duration = Duration::from_secs(15);
static DONE: &[u8] = include_bytes!("../assets/sounds/done.mp3");
static REQUEST: &[u8] = include_bytes!("../assets/sounds/request.mp3");
static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sound {
    /// An agent finished its turn where the viewer was not looking.
    Done,
    /// An agent needs the person: a permission or a question.
    Request,
}

/// The sound for one of Herdr's attention alerts.
pub fn for_alert(alert: Alert) -> Sound {
    match alert {
        Alert::Blocked => Sound::Request,
        Alert::Done => Sound::Done,
    }
}

/// Plays a sound in the background; a missing player or a failure stays silent.
pub fn play(sound: Sound, custom: Option<PathBuf>) {
    if std::env::var_os(DISABLE_ENV).is_some() {
        return;
    }
    std::thread::spawn(move || {
        if custom
            .as_deref()
            .is_some_and(|path| run_player(path).is_ok())
        {
            return;
        }
        let data = match sound {
            Sound::Done => DONE,
            Sound::Request => REQUEST,
        };
        let path = std::env::temp_dir().join(format!(
            "herdr-tui-sound-{}-{}.mp3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        if std::fs::File::create(&path)
            .and_then(|mut file| file.write_all(data))
            .is_ok()
        {
            let _ = run_player(&path);
        }
        let _ = std::fs::remove_file(&path);
    });
}

#[cfg(target_os = "macos")]
fn players() -> &'static [(&'static str, &'static [&'static str])] {
    &[("afplay", &[])]
}

/// MP3-capable players only: bare aplay would play the bytes as raw PCM.
#[cfg(not(target_os = "macos"))]
fn players() -> &'static [(&'static str, &'static [&'static str])] {
    &[
        ("paplay", &[]),
        ("pw-play", &[]),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet"]),
        ("mpg123", &["-q"]),
        ("mpv", &["--no-video", "--really-quiet"]),
    ]
}

fn run_player(path: &Path) -> Result<(), ()> {
    for (program, args) in players() {
        let Ok(mut child) = Command::new(program)
            .args(*args)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        let deadline = Instant::now() + PLAYER_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(_)) | Err(_) => break,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            }
        }
    }
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_client_lib::model::alert;

    #[test]
    fn questions_always_sound_and_finishes_only_when_unwatched() {
        let sound = |prev, new, watched| alert(prev, new, watched).map(for_alert);
        assert_eq!(
            sound(Some("working"), Some("blocked"), true),
            Some(Sound::Request)
        );
        assert_eq!(sound(None, Some("blocked"), false), Some(Sound::Request));
        assert_eq!(
            sound(Some("working"), Some("idle"), false),
            Some(Sound::Done)
        );
        assert_eq!(
            sound(Some("blocked"), Some("done"), false),
            Some(Sound::Done)
        );
        assert_eq!(sound(Some("working"), Some("idle"), true), None);
        assert_eq!(sound(None, Some("idle"), false), None);
        assert_eq!(sound(Some("idle"), Some("idle"), false), None);
        assert_eq!(sound(Some("blocked"), Some("blocked"), false), None);
    }

    #[test]
    fn embedded_sounds_are_mp3() {
        for data in [DONE, REQUEST] {
            assert!(data.starts_with(b"ID3") || data[0] == 0xff, "not an mp3");
        }
    }
}
