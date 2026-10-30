//! Herdr's default keymap and keybind help, adapted from Herdr `src/config/model.rs`
//! and `src/input/keybind_help.rs`.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeMap;

/// Actions use Herdr's `[keys]` names so its documentation applies.
pub const DEFAULTS: &[(&str, &str)] = &[
    ("prefix", "ctrl+b"),
    ("help", "prefix+?"),
    ("settings", "prefix+s"),
    ("new_workspace", "prefix+shift+n"),
    ("new_worktree", "prefix+shift+g"),
    ("open_worktree", ""),
    ("remove_worktree", ""),
    ("rename_workspace", "prefix+shift+w"),
    ("close_workspace", "prefix+shift+d"),
    ("workspace_picker", "prefix+w"),
    ("goto", "prefix+g"),
    ("detach", "prefix+q"),
    ("reload_config", "prefix+shift+r"),
    ("open_notification_target", "prefix+o"),
    ("previous_workspace", ""),
    ("next_workspace", ""),
    ("previous_agent", ""),
    ("next_agent", ""),
    ("new_tab", "prefix+c"),
    ("rename_tab", "prefix+shift+t"),
    ("previous_tab", "prefix+p"),
    ("next_tab", "prefix+n"),
    ("move_tab_previous", ""),
    ("move_tab_next", ""),
    ("close_tab", "prefix+shift+x"),
    ("rename_pane", "prefix+shift+p"),
    ("edit_scrollback", "prefix+e"),
    ("clear_pane", ""),
    ("copy_mode", "prefix+["),
    ("focus_pane_left", "prefix+h"),
    ("focus_pane_down", "prefix+j"),
    ("focus_pane_up", "prefix+k"),
    ("focus_pane_right", "prefix+l"),
    ("swap_pane_left", "prefix+shift+h"),
    ("swap_pane_down", "prefix+shift+j"),
    ("swap_pane_up", "prefix+shift+k"),
    ("swap_pane_right", "prefix+shift+l"),
    ("cycle_pane_next", "prefix+tab"),
    ("cycle_pane_previous", "prefix+shift+tab"),
    ("last_pane", ""),
    ("split_vertical", "prefix+v"),
    ("split_horizontal", "prefix+minus"),
    ("close_pane", "prefix+x"),
    ("zoom", "prefix+z"),
    ("resize_mode", "prefix+r"),
    ("resize_pane_left", ""),
    ("resize_pane_down", ""),
    ("resize_pane_up", ""),
    ("resize_pane_right", ""),
    ("toggle_sidebar", "prefix+b"),
];

/// Actions bound to a digit range, such as `switch_tab = "prefix+1..9"`; each
/// digit runs `action:N`. Herdr's `[keys.indexed]` modifier combos add direct ranges.
pub const RANGES: &[(&str, &str, &str)] = &[
    ("switch_tab", "prefix+1..9", "indexed.tabs"),
    ("switch_workspace", "", "indexed.workspaces"),
    ("focus_agent", "", "indexed.agents"),
];

/// Keys inside workspace navigation (prefix+w), Herdr's `navigate_*` names.
pub const NAVIGATION: &[(&str, &str)] = &[
    ("navigate_workspace_up", "up"),
    ("navigate_workspace_down", "down"),
    ("navigate_pane_left", "h"),
    ("navigate_pane_down", "j"),
    ("navigate_pane_up", "k"),
    ("navigate_pane_right", "l"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub prefix: bool,
    pub modifiers: KeyModifiers,
}

/// Parses `prefix+1..9`, `alt+1..9`, or an indexed modifier combo such as `alt`.
fn parse_range(text: &str, indexed: bool) -> Option<Range> {
    let text = text.trim().to_lowercase();
    let (prefix, rest) = match text.strip_prefix("prefix+") {
        Some(rest) => (true, rest.to_owned()),
        None => (false, text.clone()),
    };
    let mut modifiers = KeyModifiers::empty();
    let mut saw_range = indexed;
    for part in rest.split('+').filter(|p| !p.is_empty()) {
        match part.trim() {
            "1..9" if !saw_range => saw_range = true,
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            "alt" | "option" | "meta" => modifiers |= KeyModifiers::ALT,
            _ => return None,
        }
    }
    (saw_range && (prefix || !modifiers.is_empty())).then_some(Range { prefix, modifiers })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Combo {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub prefix: bool,
    pub combo: Combo,
}

/// Folds the ways hosts report one key into one form: letters lower-case with
/// Shift, other printable characters as typed without Shift.
pub fn normalize(code: KeyCode, modifiers: KeyModifiers) -> Combo {
    let mut modifiers =
        modifiers & (KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT);
    let code = match code {
        KeyCode::Char(c) if c.is_ascii_uppercase() => {
            modifiers |= KeyModifiers::SHIFT;
            KeyCode::Char(c.to_ascii_lowercase())
        }
        KeyCode::Char(c) if !c.is_alphanumeric() => {
            modifiers -= KeyModifiers::SHIFT;
            KeyCode::Char(c)
        }
        KeyCode::BackTab => {
            modifiers |= KeyModifiers::SHIFT;
            KeyCode::Tab
        }
        other => other,
    };
    Combo { code, modifiers }
}

pub fn combo(key: &KeyEvent) -> Combo {
    normalize(key.code, key.modifiers)
}

fn parse_key(text: &str) -> Option<Combo> {
    let mut modifiers = KeyModifiers::empty();
    let mut parts: Vec<&str> = text.split('+').collect();
    // "ctrl++" names the plus key.
    if text.ends_with("++") {
        parts.truncate(parts.len().saturating_sub(2));
        parts.push("+");
    }
    let key = parts.pop()?;
    for part in parts {
        match part {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            "alt" | "option" | "meta" => modifiers |= KeyModifiers::ALT,
            _ => return None,
        }
    }
    let code = match key {
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "space" => KeyCode::Char(' '),
        "minus" => KeyCode::Char('-'),
        "plus" => KeyCode::Char('+'),
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        key if key.chars().count() == 1 => KeyCode::Char(key.chars().next()?),
        key if key.starts_with('f') => KeyCode::F(key[1..].parse().ok()?),
        _ => return None,
    };
    Some(normalize(code, modifiers))
}

pub fn parse(text: &str) -> Option<Binding> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return None;
    }
    match text.strip_prefix("prefix+") {
        Some(rest) => Some(Binding {
            prefix: true,
            combo: parse_key(rest)?,
        }),
        None => Some(Binding {
            prefix: false,
            combo: parse_key(&text)?,
        }),
    }
}

pub fn key_label(combo: Combo) -> String {
    let mut parts = vec![];
    if combo.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl".to_owned());
    }
    if combo.modifiers.contains(KeyModifiers::ALT) {
        parts.push("alt".to_owned());
    }
    if combo.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("shift".to_owned());
    }
    parts.push(match combo.code {
        KeyCode::Char('-') => "minus".into(),
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::F(n) => format!("f{n}"),
        other => format!("{other:?}").to_lowercase(),
    });
    parts.join("+")
}

impl Binding {
    pub fn label(&self) -> String {
        if self.prefix {
            format!("prefix+{}", key_label(self.combo))
        } else {
            key_label(self.combo)
        }
    }
}

/// The resolved keymap: Herdr's defaults with the user's overrides.
#[derive(Clone, Debug)]
pub struct Keymap {
    pub prefix: Combo,
    bindings: Vec<(String, Binding)>,
    ranges: Vec<(String, Range)>,
    navigation: Vec<(String, Combo)>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new(&BTreeMap::new())
    }
}

impl Keymap {
    pub fn new(overrides: &BTreeMap<String, String>) -> Self {
        let value = |action: &str, default: &str| {
            overrides
                .get(action)
                .cloned()
                .unwrap_or_else(|| default.to_owned())
        };
        let prefix = parse(&value("prefix", "ctrl+b"))
            .filter(|binding| !binding.prefix)
            .map_or(normalize(KeyCode::Char('b'), KeyModifiers::CONTROL), |b| {
                b.combo
            });
        let mut bindings: Vec<(String, Binding)> = vec![];
        for (action, default) in DEFAULTS.iter().skip(1) {
            if let Some(binding) = parse(&value(action, default))
                && !bindings.iter().any(|(_, existing)| *existing == binding)
                && !(binding.prefix && binding.combo == prefix)
            {
                bindings.push(((*action).to_owned(), binding));
            }
        }
        let mut ranges = vec![];
        for (action, default, indexed) in RANGES {
            for (text, indexed) in [(value(action, default), false), (value(indexed, ""), true)] {
                if let Some(range) = parse_range(&text, indexed)
                    && !ranges.iter().any(|(_, existing)| *existing == range)
                {
                    ranges.push(((*action).to_owned(), range));
                }
            }
        }
        let navigation = NAVIGATION
            .iter()
            .filter_map(|(action, default)| {
                let binding = parse(&value(action, default)).filter(|b| !b.prefix)?;
                Some(((*action).to_owned(), binding.combo))
            })
            .collect();
        Self {
            prefix,
            bindings,
            ranges,
            navigation,
        }
    }

    pub fn prefix_label(&self) -> String {
        key_label(self.prefix)
    }

    pub fn label(&self, action: &str) -> String {
        if let Some((_, range)) = self.ranges.iter().find(|(name, _)| name == action) {
            let combo = Combo {
                code: KeyCode::Char('1'),
                modifiers: range.modifiers,
            };
            let keys = key_label(combo).replace('1', "1..9");
            return if range.prefix {
                format!("prefix+{keys}")
            } else {
                keys
            };
        }
        if let Some((_, combo)) = self.navigation.iter().find(|(name, _)| name == action) {
            return key_label(*combo);
        }
        self.bindings
            .iter()
            .find(|(name, _)| name == action)
            .map_or_else(|| "unset".to_owned(), |(_, binding)| binding.label())
    }
    /// A digit key on a range binding, as `action:N`.
    fn range(&self, combo: Combo, prefix: bool) -> Option<String> {
        let KeyCode::Char(c @ '1'..='9') = combo.code else {
            return None;
        };
        self.ranges
            .iter()
            .find(|(_, range)| range.prefix == prefix && range.modifiers == combo.modifiers)
            .map(|(action, _)| format!("{action}:{c}"))
    }
    /// The `navigate_*` action a key runs inside workspace navigation.
    pub fn navigation(&self, key: &KeyEvent) -> Option<&str> {
        let combo = combo(key);
        self.navigation
            .iter()
            .find(|(_, c)| *c == combo)
            .map(|(action, _)| action.as_str())
    }

    /// The key after the prefix, with range digits as `action:N`.
    pub fn prefixed(&self, key: &KeyEvent) -> Option<String> {
        let combo = combo(key);
        if let Some((action, _)) = self
            .bindings
            .iter()
            .find(|(_, binding)| binding.prefix && binding.combo == combo)
        {
            return Some(action.clone());
        }
        self.range(combo, true)
    }

    pub fn direct(&self, key: &KeyEvent) -> Option<String> {
        let combo = combo(key);
        self.bindings
            .iter()
            .find(|(_, binding)| !binding.prefix && binding.combo == combo)
            .map(|(action, _)| action.clone())
            .or_else(|| self.range(combo, false))
    }

    pub fn is_prefix(&self, key: &KeyEvent) -> bool {
        combo(key) == self.prefix
    }
}

pub type HelpGroup = (&'static str, Vec<(String, &'static str)>);

pub fn help_groups(keys: &Keymap) -> Vec<HelpGroup> {
    let entry = |action: &str, label: &'static str| (keys.label(action), label);
    vec![
        (
            "global",
            vec![
                (keys.prefix_label(), "prefix mode"),
                entry("help", "keybinds"),
                entry("settings", "settings"),
                entry("detach", "detach"),
                entry("reload_config", "reload config"),
                entry("open_notification_target", "go to notification"),
            ],
        ),
        (
            "navigation",
            vec![
                ("esc".into(), "back"),
                (
                    format!(
                        "{} / {}",
                        keys.label("navigate_workspace_up"),
                        keys.label("navigate_workspace_down")
                    ),
                    "workspace list",
                ),
                (
                    format!(
                        "{} / {} / {} / {}",
                        keys.label("navigate_pane_left"),
                        keys.label("navigate_pane_down"),
                        keys.label("navigate_pane_up"),
                        keys.label("navigate_pane_right")
                    ),
                    "move focus",
                ),
                ("tab / shift+tab".into(), "cycle pane"),
                ("enter".into(), "open workspace"),
                ("1..9".into(), "switch workspace"),
            ],
        ),
        (
            "workspaces / tabs",
            vec![
                entry("workspace_picker", "workspace navigation"),
                entry("goto", "session navigator"),
                entry("new_workspace", "new workspace"),
                entry("new_worktree", "new worktree"),
                entry("open_worktree", "open worktree"),
                entry("remove_worktree", "delete worktree checkout"),
                entry("rename_workspace", "rename workspace"),
                entry("close_workspace", "close workspace"),
                entry("previous_workspace", "previous workspace"),
                entry("next_workspace", "next workspace"),
                entry("previous_agent", "previous agent"),
                entry("next_agent", "next agent"),
                entry("new_tab", "new tab"),
                entry("rename_tab", "rename tab"),
                entry("previous_tab", "previous tab"),
                entry("next_tab", "next tab"),
                entry("move_tab_previous", "move tab left"),
                entry("move_tab_next", "move tab right"),
                entry("switch_tab", "switch tab 1-9"),
                entry("switch_workspace", "switch workspace 1-9"),
                entry("focus_agent", "focus agent 1-9"),
                entry("close_tab", "close tab"),
            ],
        ),
        (
            "panes",
            vec![
                entry("split_vertical", "split vertical"),
                entry("split_horizontal", "split horizontal"),
                entry("close_pane", "close pane"),
                entry("rename_pane", "rename pane"),
                entry("edit_scrollback", "edit scrollback in $EDITOR"),
                entry("clear_pane", "clear pane"),
                entry("copy_mode", "copy mode"),
                entry("zoom", "zoom pane"),
                entry("resize_mode", "resize mode"),
                entry("resize_pane_left", "resize pane left"),
                entry("resize_pane_down", "resize pane down"),
                entry("resize_pane_up", "resize pane up"),
                entry("resize_pane_right", "resize pane right"),
                entry("toggle_sidebar", "toggle sidebar"),
                entry("focus_pane_left", "focus pane left"),
                entry("focus_pane_down", "focus pane down"),
                entry("focus_pane_up", "focus pane up"),
                entry("focus_pane_right", "focus pane right"),
                entry("swap_pane_left", "swap pane left"),
                entry("swap_pane_down", "swap pane down"),
                entry("swap_pane_up", "swap pane up"),
                entry("swap_pane_right", "swap pane right"),
                entry("cycle_pane_next", "cycle pane next"),
                entry("cycle_pane_previous", "cycle pane previous"),
                entry("last_pane", "last pane"),
            ],
        ),
    ]
}

pub fn filter_help_groups(groups: Vec<HelpGroup>, query: &str) -> Vec<HelpGroup> {
    if query.is_empty() {
        return groups;
    }
    let query = query.to_lowercase();
    groups
        .into_iter()
        .filter_map(|(group, entries)| {
            let entries: Vec<_> = entries
                .into_iter()
                .filter(|(key, label)| {
                    key.to_lowercase().contains(&query) || label.to_lowercase().contains(&query)
                })
                .collect();
            (!entries.is_empty()).then_some((group, entries))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new_with_kind(code, modifiers, KeyEventKind::Press)
    }

    #[test]
    fn defaults_match_herdr_and_host_spellings() {
        let keys = Keymap::default();
        assert!(keys.is_prefix(&key(KeyCode::Char('b'), KeyModifiers::CONTROL)));
        assert_eq!(
            keys.prefixed(&key(KeyCode::Char('N'), KeyModifiers::SHIFT))
                .as_deref(),
            Some("new_workspace")
        );
        assert_eq!(
            keys.prefixed(&key(KeyCode::Char('N'), KeyModifiers::empty()))
                .as_deref(),
            Some("new_workspace")
        );
        assert_eq!(
            keys.prefixed(&key(KeyCode::Char('?'), KeyModifiers::SHIFT))
                .as_deref(),
            Some("help")
        );
        assert_eq!(
            keys.prefixed(&key(KeyCode::Char('-'), KeyModifiers::empty()))
                .as_deref(),
            Some("split_horizontal")
        );
        assert_eq!(
            keys.prefixed(&key(KeyCode::BackTab, KeyModifiers::SHIFT))
                .as_deref(),
            Some("cycle_pane_previous")
        );
        assert_eq!(
            keys.prefixed(&key(KeyCode::Char('3'), KeyModifiers::empty()))
                .as_deref(),
            Some("switch_tab:3")
        );
        assert_eq!(keys.label("new_workspace"), "prefix+shift+n");
        assert_eq!(keys.label("next_agent"), "unset");
    }

    #[test]
    fn overrides_replace_defaults_and_later_duplicates_are_dropped() {
        let keys = Keymap::new(&BTreeMap::from([
            ("next_agent".to_owned(), "alt+a".to_owned()),
            ("new_tab".to_owned(), "prefix+n".to_owned()),
        ]));
        assert_eq!(
            keys.direct(&key(KeyCode::Char('a'), KeyModifiers::ALT))
                .as_deref(),
            Some("next_agent")
        );
        assert_eq!(
            keys.prefixed(&key(KeyCode::Char('n'), KeyModifiers::empty()))
                .as_deref(),
            Some("new_tab")
        );
        assert_eq!(keys.label("next_tab"), "unset");
    }

    #[test]
    fn ranges_and_indexed_modifiers_run_numbered_actions() {
        let keys = Keymap::new(&BTreeMap::from([
            ("switch_workspace".to_owned(), "prefix+alt+1..9".to_owned()),
            ("indexed.agents".to_owned(), "alt".to_owned()),
            ("navigate_pane_left".to_owned(), "a".to_owned()),
        ]));
        assert_eq!(
            keys.prefixed(&key(KeyCode::Char('3'), KeyModifiers::empty()))
                .as_deref(),
            Some("switch_tab:3")
        );
        assert_eq!(
            keys.prefixed(&key(KeyCode::Char('2'), KeyModifiers::ALT))
                .as_deref(),
            Some("switch_workspace:2")
        );
        assert_eq!(
            keys.direct(&key(KeyCode::Char('4'), KeyModifiers::ALT))
                .as_deref(),
            Some("focus_agent:4")
        );
        assert_eq!(
            keys.direct(&key(KeyCode::Char('4'), KeyModifiers::empty())),
            None
        );
        assert_eq!(keys.label("switch_workspace"), "prefix+alt+1..9");
        assert_eq!(keys.label("focus_agent"), "alt+1..9");
        assert_eq!(keys.label("settings"), "prefix+s");
        assert_eq!(
            keys.navigation(&key(KeyCode::Char('a'), KeyModifiers::empty())),
            Some("navigate_pane_left")
        );
        assert_eq!(
            keys.navigation(&key(KeyCode::Char('h'), KeyModifiers::empty())),
            None
        );
    }
}
