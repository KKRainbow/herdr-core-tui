use crate::Result;
use herdr_client_lib::view::{Attention, View};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
/// Settings read from Herdr's `config.toml` layout.
pub struct Preferences {
    pub sidebar_width: u16,
    /// Herdr theme name, such as `catppuccin` or `tokyo-night`.
    pub theme: String,
    /// Herdr's `[theme.custom]` color tokens over the theme.
    pub custom_colors: BTreeMap<String, String>,
    /// Herdr's legacy `ui.accent`; applies when not `cyan` and no custom accent is set.
    pub accent: String,
    /// `dots` or `symbols`, as Herdr's `ui.status_indicators`.
    pub status_indicators: String,
    /// `spaces` or `priority`, as Herdr's `ui.agent_panel_sort`.
    pub agent_panel_sort: String,
    /// Herdr `[keys]` names mapped to bindings, such as `new_tab = "prefix+c"`;
    /// `[keys.indexed]` entries appear as `indexed.tabs` and so on.
    pub keys: BTreeMap<String, Vec<String>>,
    pub sound: Sound,
    pub worktrees: Worktrees,
    /// Herdr's `[[keys.command]]`: keys that run a command.
    pub commands: Vec<Command>,
    /// Herdr's other `[ui]` options this client applies.
    pub ui: Ui,
    /// Herdr's `[terminal] default_shell`: the program new panes run; empty uses core's shell.
    pub default_shell: String,
}
/// Herdr's `pane_borders`: `auto` frames split panes, `always` a lone pane too, `off` none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaneBorders {
    #[default]
    Auto,
    Always,
    Off,
}
impl PaneBorders {
    pub fn draws_borders(self) -> bool {
        !matches!(self, Self::Off)
    }
    pub fn shows_borders(self, multi_pane: bool) -> bool {
        self.draws_borders() && (multi_pane || matches!(self, Self::Always))
    }
}
impl<'de> Deserialize<'de> for PaneBorders {
    /// Herdr's words, or a legacy boolean: true is auto, false is off.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(match toml::Value::deserialize(d)? {
            toml::Value::Boolean(true) => Self::Auto,
            toml::Value::Boolean(false) => Self::Off,
            toml::Value::String(word) if word.eq_ignore_ascii_case("always") => Self::Always,
            toml::Value::String(word) if word.eq_ignore_ascii_case("off") => Self::Off,
            _ => Self::Auto,
        })
    }
}
/// Herdr's `[ui]` options beyond theme, indicators, sorting, and sound, with Herdr's defaults.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct Ui {
    pub sidebar_min_width: u16,
    pub sidebar_max_width: u16,
    pub sidebar_start_collapsed: bool,
    /// `compact` keeps a narrow strip when collapsed; `hidden` removes it.
    pub sidebar_collapsed_mode: String,
    pub mouse_capture: bool,
    pub copy_on_select: bool,
    pub mouse_scroll_lines: Option<usize>,
    pub confirm_close: bool,
    pub prompt_new_tab_name: bool,
    pub prompt_new_workspace_name: bool,
    pub pane_borders: PaneBorders,
    pub pane_outer_borders: bool,
    pub pane_scrollbars: bool,
    pub pane_gaps: bool,
    pub show_agent_labels_on_pane_borders: bool,
    pub hide_tab_bar_when_single_tab: bool,
    /// `top` or `bottom`.
    pub tab_bar_position: String,
    /// Outer terminal title; tokens `{hostname}`, `{workspace}`, `{tab}`, `{pane}`,
    /// `{terminal_title}`. Empty leaves the title alone.
    pub window_title: String,
    pub toast: Toast,
}
/// Herdr's `[ui.toast]`: where agent notifications go, after an optional delay.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct Toast {
    /// `off`, `herdr` (a card inside the TUI), `terminal`, or `system`.
    pub delivery: String,
    pub delay_seconds: u64,
    pub herdr: HerdrToast,
}
impl Default for Toast {
    fn default() -> Self {
        Self {
            delivery: "off".into(),
            delay_seconds: 0,
            herdr: HerdrToast::default(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct HerdrToast {
    /// `top-left`, `top-right`, `bottom-left`, or `bottom-right`.
    pub position: String,
}
impl Default for HerdrToast {
    fn default() -> Self {
        Self {
            position: "bottom-right".into(),
        }
    }
}
impl Default for Ui {
    fn default() -> Self {
        Self {
            sidebar_min_width: 18,
            sidebar_max_width: 36,
            sidebar_start_collapsed: false,
            sidebar_collapsed_mode: "compact".into(),
            mouse_capture: true,
            copy_on_select: true,
            mouse_scroll_lines: None,
            confirm_close: true,
            prompt_new_tab_name: true,
            prompt_new_workspace_name: false,
            pane_borders: PaneBorders::Auto,
            pane_outer_borders: true,
            pane_scrollbars: true,
            pane_gaps: true,
            show_agent_labels_on_pane_borders: false,
            hide_tab_bar_when_single_tab: false,
            tab_bar_position: "top".into(),
            window_title: "{hostname}: {workspace}".into(),
            toast: Toast::default(),
        }
    }
}
impl Ui {
    /// Lines a wheel notch scrolls, Herdr's default 3.
    pub fn scroll_lines(&self) -> usize {
        self.mouse_scroll_lines.filter(|n| *n > 0).unwrap_or(3)
    }
    pub fn tab_bar_bottom(&self) -> bool {
        self.tab_bar_position.eq_ignore_ascii_case("bottom")
    }
    pub fn sidebar_hidden_when_collapsed(&self) -> bool {
        self.sidebar_collapsed_mode.eq_ignore_ascii_case("hidden")
    }
}
/// One custom command binding, as Herdr's `CommandKeybindConfig`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Command {
    /// `prefix+g` for prefix mode, or a modified chord such as `alt+g`.
    pub key: String,
    pub command: String,
    #[serde(rename = "type")]
    pub kind: CommandKind,
    pub description: Option<String>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    /// Runs in the background through the shell.
    #[default]
    Shell,
    /// Runs in a new zoomed pane beside the focused one.
    Pane,
    /// Herdr's popup pane; here a zoomed pane as for `pane`.
    Popup,
    /// Herdr plugin actions; plugins are not part of this client.
    PluginAction,
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
    /// Replaces every built-in sound.
    pub path: Option<PathBuf>,
    /// Replaces the built-in sound an agent makes when it finishes.
    pub done_path: Option<PathBuf>,
    /// Replaces the built-in sound an agent makes when it asks a question.
    pub request_path: Option<PathBuf>,
    /// The folder relative paths start from: that of the file they came from.
    #[serde(skip)]
    pub folder: Option<PathBuf>,
}
impl Default for Sound {
    fn default() -> Self {
        Self {
            enabled: true,
            path: None,
            done_path: None,
            request_path: None,
            folder: None,
        }
    }
}
impl Sound {
    /// The replacement file, with relative paths resolved from the config folder.
    pub fn path(&self, sound: crate::sound::Sound) -> Option<PathBuf> {
        let path = match sound {
            crate::sound::Sound::Done => self.done_path.as_ref(),
            crate::sound::Sound::Request => self.request_path.as_ref(),
        }
        .or(self.path.as_ref())?;
        Some(match &self.folder {
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
            custom_colors: BTreeMap::new(),
            accent: "cyan".into(),
            status_indicators: "dots".into(),
            agent_panel_sort: "spaces".into(),
            keys: BTreeMap::new(),
            sound: Sound::default(),
            worktrees: Worktrees::default(),
            commands: vec![],
            ui: Ui::default(),
            default_shell: String::new(),
        }
    }
}
impl Preferences {
    pub fn keymap(&self) -> crate::shell::keys::Keymap {
        let commands: Vec<_> = self
            .commands
            .iter()
            .map(|c| (c.key.clone(), c.description.clone()))
            .collect();
        crate::shell::keys::Keymap::with_commands(&self.keys, &commands)
    }
    /// The theme with Herdr's custom colors and legacy accent applied.
    pub fn palette(&self, theme: &str) -> crate::shell::palette::Palette {
        let mut palette = crate::shell::palette::Palette::from_name(theme).unwrap_or_default();
        palette.apply_overrides(&self.custom_colors);
        if self.accent != "cyan" && !self.custom_colors.contains_key("accent") {
            palette.accent = crate::shell::palette::parse_color(&self.accent);
        }
        palette
    }
    pub fn shell(&self) -> crate::shell::Config {
        crate::shell::Config {
            palette: self.palette(&self.theme),
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
/// This client's `config.toml`, in Herdr's format.
pub fn config_path() -> PathBuf {
    base("XDG_CONFIG_HOME", ".config").join("config.toml")
}
/// Herdr's own `config.toml`, read when this client has none yet.
pub fn herdr_config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("HERDR_CONFIG_PATH") {
        return path.into();
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".config")
        })
        .join("herdr")
        .join("config.toml")
}
/// The file settings come from: this client's, else Herdr's, else none.
pub fn source_path() -> Option<PathBuf> {
    [config_path(), herdr_config_path()]
        .into_iter()
        .find(|path| path.is_file())
}
/// The sections of Herdr's `config.toml` this client reads; others are ignored.
#[derive(Default, Deserialize)]
#[serde(default)]
struct File {
    theme: ThemeFile,
    ui: UiFile,
    terminal: TerminalFile,
    keys: KeysFile,
    worktrees: Worktrees,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ThemeFile {
    name: Option<String>,
    /// Token colors; Herdr's `light`/`dark` tables are skipped.
    custom: BTreeMap<String, toml::Value>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct TerminalFile {
    default_shell: String,
}
#[derive(Deserialize)]
#[serde(default)]
struct UiFile {
    sidebar_width: u16,
    accent: String,
    status_indicators: String,
    agent_panel_sort: String,
    sound: Sound,
    #[serde(flatten)]
    options: Ui,
}
impl Default for UiFile {
    fn default() -> Self {
        let defaults = Preferences::default();
        Self {
            sidebar_width: defaults.sidebar_width,
            accent: defaults.accent,
            status_indicators: defaults.status_indicators,
            agent_panel_sort: defaults.agent_panel_sort,
            sound: Sound::default(),
            options: Ui::default(),
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct KeysFile {
    command: Vec<Command>,
    indexed: BTreeMap<String, String>,
    /// Each action takes one binding or a list, as Herdr's `BindingConfig`.
    #[serde(flatten)]
    actions: BTreeMap<String, toml::Value>,
}
impl Preferences {
    /// Settings from Herdr-format TOML text.
    pub fn parse(text: &str, folder: Option<PathBuf>) -> Result<Self> {
        let file: File = toml::from_str(text)?;
        let mut keys = BTreeMap::new();
        for (action, value) in file.keys.actions {
            let bindings = match value {
                toml::Value::String(binding) => vec![binding],
                toml::Value::Array(list) => list
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                _ => continue,
            };
            keys.insert(action, bindings);
        }
        for (kind, modifiers) in file.keys.indexed {
            keys.insert(format!("indexed.{kind}"), vec![modifiers]);
        }
        let mut sound = file.ui.sound;
        sound.folder = folder;
        Ok(Self {
            sidebar_width: file.ui.sidebar_width,
            theme: file.theme.name.unwrap_or_else(|| "catppuccin".into()),
            custom_colors: file
                .theme
                .custom
                .into_iter()
                .filter_map(|(token, value)| Some((token, value.as_str()?.to_owned())))
                .collect(),
            accent: file.ui.accent,
            status_indicators: file.ui.status_indicators,
            agent_panel_sort: file.ui.agent_panel_sort,
            keys,
            sound,
            worktrees: file.worktrees,
            commands: file.keys.command,
            ui: file.ui.options,
            default_shell: file.terminal.default_shell,
        })
    }
}
/// Writes one of Herdr's settings edits to this client's `config.toml`. The first
/// edit starts from Herdr's file when this client has none, so its settings carry over.
pub fn edit(edit: crate::config_edit::Edit<'_>) -> Result<()> {
    let seed = herdr_config_path();
    crate::config_edit::update_file_at(&config_path(), Some(&seed), |content| edit.apply(content))?;
    Ok(())
}
pub fn load() -> Result<Preferences> {
    let Some(path) = source_path() else {
        return Ok(Preferences::default());
    };
    let text = std::fs::read_to_string(&path)?;
    Preferences::parse(
        text.trim_start_matches('\u{feff}'),
        path.parent().map(PathBuf::from),
    )
    .map_err(|e| format!("{}: {e}", path.display()).into())
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
/// Seen completions this viewer keeps between runs, like the app's saved attention.
pub fn read_attention(session: &str) -> Attention {
    std::fs::read(state_path("attention", session))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
pub fn save_attention(session: &str, attention: &Attention) -> Result<()> {
    write_atomic(
        &state_path("attention", session),
        &serde_json::to_vec(attention)?,
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_herdr_config_sections() {
        let text = r##"
onboarding = false
[theme]
name = "nord"
[theme.custom]
accent = "#ff0000"
[theme.custom.dark]
accent = "#00ff00"
[ui]
sidebar_width = 30
status_indicators = "symbols"
[ui.sound]
enabled = false
path = "all.mp3"
[keys]
prefix = "ctrl+a"
new_tab = ["prefix+c", "prefix+t"]
[keys.indexed]
tabs = "alt"
[[keys.command]]
key = "prefix+g"
command = "lazygit"
type = "pane"
[worktrees]
directory = "~/trees"
[remote]
unknown = 1
"##;
        let p = Preferences::parse(text, Some("/conf".into())).unwrap();
        assert_eq!((p.theme.as_str(), p.sidebar_width), ("nord", 30));
        assert_eq!(p.custom_colors["accent"], "#ff0000");
        assert_eq!(p.status_indicators, "symbols");
        assert_eq!(p.keys["new_tab"], ["prefix+c", "prefix+t"]);
        assert_eq!(p.keys["indexed.tabs"], ["alt"]);
        assert_eq!(p.commands[0].kind, CommandKind::Pane);
        assert_eq!(p.worktrees.directory.as_deref(), Some("~/trees"));
        assert!(!p.sound.enabled);
        assert_eq!(
            p.sound.path(crate::sound::Sound::Done),
            Some(PathBuf::from("/conf/all.mp3"))
        );
        let palette = p.palette(&p.theme);
        assert_eq!(palette.accent, ratatui::style::Color::Rgb(255, 0, 0));
        assert_eq!(p.ui, Ui::default());
        let keys = p.keymap();
        assert_eq!(keys.label("new_tab"), "prefix+c");
        assert_eq!(keys.prefix_label(), "ctrl+a");
    }

    #[test]
    fn ui_options_follow_herdr_including_legacy_booleans() {
        let p = Preferences::parse(
            "[ui]\npane_borders = false\ntab_bar_position = \"bottom\"\nconfirm_close = false\nmouse_scroll_lines = 5\n[terminal]\ndefault_shell = \"fish\"\n",
            None,
        )
        .unwrap();
        assert_eq!(p.ui.pane_borders, PaneBorders::Off);
        assert!(p.ui.tab_bar_bottom() && !p.ui.confirm_close);
        assert_eq!(p.ui.scroll_lines(), 5);
        assert_eq!(p.default_shell, "fish");
        assert!(p.ui.prompt_new_tab_name && p.ui.copy_on_select);
    }

    #[test]
    fn legacy_accent_applies_without_a_custom_accent() {
        let p = Preferences::parse("[ui]\naccent = \"#010203\"\n", None).unwrap();
        assert_eq!(
            p.palette("catppuccin").accent,
            ratatui::style::Color::Rgb(1, 2, 3)
        );
    }
}
