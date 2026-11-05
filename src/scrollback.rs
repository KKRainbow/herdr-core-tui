//! Open a pane's scrollback in `$EDITOR`, adapted from Herdr `src/app/custom_commands.rs`
//! and `src/platform/linux.rs` at ee5bfb5041bb3e4a49d591c72308aae3c81ecdde.
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub fn write_temp_file(content: &str) -> io::Result<PathBuf> {
    let mut last_collision = None;
    for attempt in 0..16 {
        let path = unique_path(attempt);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                file.write_all(content.as_bytes())?;
                return Ok(path);
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => last_collision = Some(err),
            Err(err) => return Err(err),
        }
    }
    Err(last_collision.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "failed to create unique scrollback temp file",
        )
    }))
}

fn unique_path(attempt: u32) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "herdr-tui-scrollback-{}-{nanos}-{attempt}.txt",
        std::process::id()
    ))
}

/// `${EDITOR:-vi}` on the file through the shell, removing the file afterwards.
pub fn editor_argv(path: &Path) -> Vec<String> {
    let quoted_path = shell_quote(&path.display().to_string());
    let command = format!(
        r#"scrollback_file={quoted_path}; eval "${{EDITOR:-vi}} \"\$scrollback_file\""; status=$?; rm -f "$scrollback_file"; exit $status"#
    );
    vec!["/bin/sh".to_string(), "-c".to_string(), command]
}

pub fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(
                    ch,
                    '@' | '%' | '_' | '+' | '=' | ':' | ',' | '.' | '/' | '-'
                )
        })
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Runs the editor on the host terminal and waits for it.
pub fn edit(text: &str) -> io::Result<()> {
    let path = write_temp_file(text)?;
    let argv = editor_argv(&path);
    let status = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .status();
    let _ = fs::remove_file(&path);
    let status = status?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("editor exited with {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_argv_preserves_unix_editor_shell_semantics() {
        let argv = editor_argv(Path::new("/tmp/herdr scrollback.txt"));
        assert_eq!(argv[0], "/bin/sh");
        assert!(argv[2].contains("EDITOR:-vi"));
        assert!(argv[2].contains("'/tmp/herdr scrollback.txt'"));
    }

    #[test]
    fn temp_files_are_private_and_unique() {
        let a = write_temp_file("one").unwrap();
        let b = write_temp_file("two").unwrap();
        assert_ne!(a, b);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&a).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(fs::read_to_string(&b).unwrap(), "two");
        for path in [a, b] {
            fs::remove_file(path).unwrap();
        }
    }
}
