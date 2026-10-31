//! Line-preserving `config.toml` edits, adapted from Herdr `src/config/io.rs` and
//! `src/config/write.rs` at ee5bfb5041bb3e4a49d591c72308aae3c81ecdde.
use std::path::Path;

/// Write a key = value pair in a TOML section (creates section if missing).
pub fn upsert_section_value(content: &str, section: &str, key: &str, value: &str) -> String {
    upsert_section_raw(content, section, key, value)
}

pub fn upsert_section_bool(content: &str, section: &str, key: &str, value: bool) -> String {
    upsert_section_raw(content, section, key, &value.to_string())
}

pub fn remove_section_key(content: &str, section: &str, key: &str) -> String {
    let header = format!("[{section}]");
    let mut result = Vec::new();
    let mut in_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed == header;
            result.push(line.to_string());
            continue;
        }
        if in_section
            && (trimmed.starts_with(&format!("{key} ")) || trimmed.starts_with(&format!("{key}=")))
        {
            continue;
        }
        result.push(line.to_string());
    }
    result.join("\n") + "\n"
}

fn upsert_section_raw(content: &str, section: &str, key: &str, value: &str) -> String {
    let header = format!("[{section}]");
    let assignment = format!("{key} = {value}");
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;
    let mut found_section = false;
    let mut inserted = false;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed == header {
            found_section = true;
            result.push(line.to_string());
            i += 1;
            while i < lines.len() {
                let current = lines[i];
                let current_trimmed = current.trim();
                if current_trimmed.starts_with('[') && current_trimmed.ends_with(']') {
                    if !inserted {
                        result.push(assignment.clone());
                        inserted = true;
                    }
                    break;
                }
                if current_trimmed.starts_with(&format!("{key} "))
                    || current_trimmed.starts_with(&format!("{key}="))
                {
                    result.push(assignment.clone());
                    inserted = true;
                } else {
                    result.push(current.to_string());
                }
                i += 1;
            }
            continue;
        }
        result.push(line.to_string());
        i += 1;
    }
    if !found_section {
        if !result.is_empty() && !result.last().is_some_and(|line| line.trim().is_empty()) {
            result.push(String::new());
        }
        result.push(header);
        result.push(assignment);
    } else if !inserted {
        result.push(assignment);
    }
    result.join("\n") + "\n"
}

/// The settings Herdr's settings overlay writes.
#[derive(Clone, Copy, Debug)]
pub enum Edit<'a> {
    Theme(&'a str),
    StatusIndicators(&'a str),
    Sound(bool),
    ToastDelivery(&'a str),
}

impl Edit<'_> {
    pub fn apply(self, content: &str) -> String {
        match self {
            Self::Theme(name) => {
                let content =
                    upsert_section_value(content, "theme", "name", &format!("\"{name}\""));
                upsert_section_bool(&content, "theme", "auto_switch", false)
            }
            Self::StatusIndicators(style) => {
                upsert_section_value(content, "ui", "status_indicators", &format!("\"{style}\""))
            }
            Self::Sound(enabled) => upsert_section_bool(content, "ui.sound", "enabled", enabled),
            Self::ToastDelivery(delivery) => {
                let content = upsert_section_value(
                    content,
                    "ui.toast",
                    "delivery",
                    &format!("\"{delivery}\""),
                );
                remove_section_key(&content, "ui.toast", "enabled")
            }
        }
    }
}

/// Reads `path` (or `seed` when it does not exist yet), applies `update`, and saves it.
pub fn update_file_at(
    path: &Path,
    seed: Option<&Path>,
    update: impl FnOnce(&str) -> String,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let read = |path: &Path| match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content.trim_start_matches('\u{feff}').to_owned())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    };
    let content = match read(path)? {
        Some(content) => content,
        None => match seed {
            Some(seed) => read(seed)?.unwrap_or_default(),
            None => String::new(),
        },
    };
    std::fs::write(path, update(&content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_section_bool_adds_missing_section() {
        let updated = upsert_section_bool("", "ui.toast", "enabled", true);
        assert!(updated.contains("[ui.toast]"));
        assert!(updated.contains("enabled = true"));
    }

    #[test]
    fn remove_section_key_removes_matching_key_from_section() {
        let content =
            "[ui.toast]\nenabled = true\ndelivery = \"herdr\"\n[ui.sound]\nenabled = true\n";
        let updated = remove_section_key(content, "ui.toast", "enabled");
        assert!(!updated.contains("[ui.toast]\nenabled = true"));
        assert!(updated.contains("delivery = \"herdr\""));
        assert!(updated.contains("[ui.sound]\nenabled = true"));
    }

    #[test]
    fn edits_keep_comments_and_other_sections() {
        let content = "# mine\n[theme]\nname = \"nord\"\n\n[keys]\nprefix = \"ctrl+a\"\n";
        let updated = Edit::Theme("dracula").apply(content);
        assert!(
            updated.starts_with("# mine\n[theme]\nname = \"dracula\""),
            "{updated}"
        );
        assert!(updated.contains("auto_switch = false"));
        assert!(updated.contains("[keys]\nprefix = \"ctrl+a\""));
        assert!(toml::from_str::<toml::Value>(&updated).is_ok());
    }
}
