use crate::Result;
use herdr_client_lib::view::View;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub sidebar_width: u16,
    pub keys: BTreeMap<String, String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            sidebar_width: 24,
            keys: BTreeMap::from([
                ("h".into(), "pane.left".into()),
                ("j".into(), "pane.down".into()),
                ("k".into(), "pane.up".into()),
                ("l".into(), "pane.right".into()),
                ("S".into(), "space.next".into()),
                ("a".into(), "agent.next".into()),
                ("e".into(), "terminal.next".into()),
                ("m".into(), "menu".into()),
                ("s".into(), "space.new".into()),
                ("t".into(), "tab.new".into()),
                ("%".into(), "split.row".into()),
                ("\"".into(), "split.column".into()),
                ("x".into(), "pane.close".into()),
                ("r".into(), "pane.rename".into()),
                ("z".into(), "zoom".into()),
                ("n".into(), "tab.next".into()),
                ("p".into(), "pane.next".into()),
                ("[".into(), "copy".into()),
                ("?".into(), "help".into()),
                (":".into(), "command".into()),
                ("w".into(), "worktree.new".into()),
            ]),
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
fn view_path(session: &str) -> PathBuf {
    let name = session
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    base("XDG_STATE_HOME", ".local/state").join(format!("view-{name}.json"))
}
pub fn read_views(session: &str) -> BTreeMap<String, View> {
    std::fs::read(view_path(session))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
pub fn save_views(session: &str, views: &BTreeMap<String, View>) -> Result<()> {
    let path = view_path(session);
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, serde_json::to_vec_pretty(views)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
