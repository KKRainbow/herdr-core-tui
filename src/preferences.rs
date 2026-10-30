use crate::Result;
use herdr_client_lib::view::View;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub sidebar_width: u16,
    /// Herdr theme name, such as `catppuccin` or `tokyo-night`.
    pub theme: String,
    /// `dots` or `symbols`, as Herdr's `ui.status_indicators`.
    pub status_indicators: String,
    /// `spaces` or `priority`, as Herdr's `ui.agent_panel_sort`.
    pub agent_panel_sort: String,
    /// Herdr `[keys]` names mapped to bindings, such as `"new_tab": "prefix+c"`.
    pub keys: BTreeMap<String, String>,
    pub sound: Sound,
    pub worktrees: Worktrees,
}
/// Herdr's `[worktrees]`: where new checkouts go.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Worktrees {
    /// Root for `<repo>/<branch-slug>` checkouts; `~` starts at the machine's home.
    /// Unset uses Herdr's `~/.herdr/worktrees`.
    pub directory: Option<String>,
}
/// Herdr's `[ui.sound]`: on by default, with optional mp3 replacements.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Sound {
    pub enabled: bool,
    /// Replaces the built-in sound an agent makes when it finishes.
    pub done_path: Option<PathBuf>,
    /// Replaces the built-in sound an agent makes when it asks a question.
    pub request_path: Option<PathBuf>,
}
impl Default for Sound {
    fn default() -> Self {
        Self {
            enabled: true,
            done_path: None,
            request_path: None,
        }
    }
}
impl Sound {
    /// The replacement file, with relative paths resolved from the config folder.
    pub fn path(&self, sound: crate::sound::Sound) -> Option<PathBuf> {
        let path = match sound {
            crate::sound::Sound::Done => self.done_path.as_ref(),
            crate::sound::Sound::Request => self.request_path.as_ref(),
        }?;
        Some(match config_path().parent() {
            Some(folder) if path.is_relative() => folder.join(path),
            _ => path.clone(),
        })
    }
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            sidebar_width: 26,
            theme: "catppuccin".into(),
            status_indicators: "dots".into(),
            agent_panel_sort: "spaces".into(),
            keys: BTreeMap::new(),
            sound: Sound::default(),
            worktrees: Worktrees::default(),
        }
    }
}
impl Preferences {
    pub fn shell(&self) -> crate::shell::Config {
        crate::shell::Config {
            palette: crate::shell::palette::Palette::from_name(&self.theme).unwrap_or_default(),
            symbols: self.status_indicators.eq_ignore_ascii_case("symbols"),
            sort: if self.agent_panel_sort.eq_ignore_ascii_case("priority") {
                crate::shell::Sort::Priority
            } else {
                crate::shell::Sort::Spaces
            },
        }
    }
}
/// Interface state the viewer keeps between runs, like Herdr's client preferences.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Chrome {
    pub sidebar_width: Option<u16>,
    pub sidebar_collapsed: bool,
    pub sidebar_section_split: f32,
    pub agent_panel_sort: Option<String>,
    pub collapsed_groups: Vec<String>,
}
impl Default for Chrome {
    fn default() -> Self {
        Self {
            sidebar_width: None,
            sidebar_collapsed: false,
            sidebar_section_split: 0.5,
            agent_panel_sort: None,
            collapsed_groups: vec![],
        }
    }
}
fn base(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into())).join(fallback)
        })
        .join("herder-tui")
}
pub fn config_path() -> PathBuf {
    base("XDG_CONFIG_HOME", ".config").join("config.json")
}
pub fn load() -> Result<Preferences> {
    match std::fs::read(config_path()) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(e) => Err(e.into()),
    }
}
fn state_path(kind: &str, session: &str) -> PathBuf {
    let name = session
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    base("XDG_STATE_HOME", ".local/state").join(format!("{kind}-{name}.json"))
}
fn view_path(session: &str) -> PathBuf {
    state_path("view", session)
}
pub fn read_chrome(session: &str) -> Chrome {
    std::fs::read(state_path("chrome", session))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
pub fn save_chrome(session: &str, chrome: &Chrome) -> Result<()> {
    write_atomic(
        &state_path("chrome", session),
        &serde_json::to_vec_pretty(chrome)?,
    )
}
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
pub fn read_views(session: &str) -> BTreeMap<String, View> {
    std::fs::read(view_path(session))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
pub fn save_views(session: &str, views: &BTreeMap<String, View>) -> Result<()> {
    write_atomic(&view_path(session), &serde_json::to_vec_pretty(views)?)
}
