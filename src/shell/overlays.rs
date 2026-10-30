//! Menus, prompts, confirmations, keybind help, the session navigator, and worktree dialogs.
//! Adapted from Herdr `src/client/shell/{overlays,worktree_overlays,context_menu,global_menu}.rs`.
use super::{
    chrome::{
        ScrollMetrics, display_width, put_right_text, put_text, scrollbar_thumb, status_color,
        status_icon,
    },
    keys::{Keymap, filter_help_groups, help_groups},
    palette::Palette,
    snapshot::{Snapshot, Status},
};
use crate::editor::TextEditor;
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget, Wrap},
};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Rename,
    Close,
    NewWorktree,
    OpenWorktree,
    RemoveWorktree,
    ToggleGroup,
    NewTab,
    RenamePane,
    ClearPaneName,
    SwapWithFocusedPane,
    SplitRight,
    SplitDown,
    MovePane,
    Zoom,
    ParkTerminal,
    ClosePane,
    KeepTerminal,
    Settings,
    Keybinds,
    ReloadConfig,
    Detach,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuTarget {
    Workspace {
        workspace_id: String,
        is_git: bool,
        is_linked_worktree: bool,
        has_worktree_children: bool,
        close_group: bool,
        collapsed: bool,
    },
    Tab {
        tab_id: String,
    },
    Pane {
        pane_id: String,
        source_pane_id: Option<String>,
        has_manual_label: bool,
    },
    /// A reporting terminal without a pane.
    Terminal {
        address: String,
    },
}

#[derive(Clone, Debug)]
pub struct ContextMenu {
    pub target: MenuTarget,
    pub x: u16,
    pub y: u16,
    pub highlighted: usize,
}

impl ContextMenu {
    pub fn items(&self) -> Vec<(&'static str, MenuAction)> {
        use MenuAction as A;
        match &self.target {
            MenuTarget::Workspace { is_git: false, .. } => {
                vec![("Rename", A::Rename), ("Close", A::Close)]
            }
            MenuTarget::Workspace {
                is_linked_worktree: true,
                ..
            } => vec![
                ("Rename", A::Rename),
                ("Close", A::Close),
                ("Delete worktree checkout...", A::RemoveWorktree),
            ],
            MenuTarget::Workspace {
                has_worktree_children: false,
                ..
            } => vec![
                ("Rename", A::Rename),
                ("Close", A::Close),
                ("New worktree", A::NewWorktree),
                ("Open worktree...", A::OpenWorktree),
            ],
            MenuTarget::Workspace {
                close_group,
                collapsed,
                ..
            } => vec![
                ("Rename", A::Rename),
                (if *close_group { "Close group" } else { "Close" }, A::Close),
                ("New worktree", A::NewWorktree),
                ("Open worktree...", A::OpenWorktree),
                (
                    if *collapsed { "Expand" } else { "Collapse" },
                    A::ToggleGroup,
                ),
            ],
            MenuTarget::Tab { .. } => vec![
                ("New tab", A::NewTab),
                ("Rename", A::Rename),
                ("Close", A::Close),
            ],
            MenuTarget::Pane {
                source_pane_id,
                has_manual_label,
                ..
            } => {
                let mut items = vec![("Rename pane", A::RenamePane)];
                if *has_manual_label {
                    items.push(("Clear pane name", A::ClearPaneName));
                }
                if source_pane_id.is_some() {
                    items.push(("Swap with focused pane", A::SwapWithFocusedPane));
                }
                items.extend([
                    ("Split right", A::SplitRight),
                    ("Split down", A::SplitDown),
                    ("Move pane...", A::MovePane),
                    ("Zoom", A::Zoom),
                    ("Park terminal", A::ParkTerminal),
                    ("Close pane", A::ClosePane),
                ]);
                items
            }
            MenuTarget::Terminal { .. } => vec![("Keep in a new tab", A::KeepTerminal)],
        }
    }
}

pub fn global_menu_items() -> Vec<(&'static str, MenuAction)> {
    vec![
        ("settings", MenuAction::Settings),
        ("keybinds", MenuAction::Keybinds),
        ("reload config", MenuAction::ReloadConfig),
        ("detach", MenuAction::Detach),
    ]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameTarget {
    Space(String),
    Tab(String),
    Pane(String),
    /// Names a new tab before it starts, as Herdr's `prompt_new_tab_name` does.
    NewTab(String),
}

#[derive(Clone, Debug)]
pub struct Rename {
    pub title: &'static str,
    pub target: RenameTarget,
    pub input: TextEditor,
}

#[derive(Clone, Debug)]
pub struct Confirm {
    pub title: String,
    pub detail: String,
    pub command: String,
    pub args: Value,
}

#[derive(Clone, Debug)]
pub struct PaneMove {
    pub pane_id: String,
    pub machine: String,
    pub destination: String,
    pub error: Option<String>,
}

pub fn pane_move_destinations<'a>(
    snapshot: &'a Snapshot,
    pane: &str,
) -> Vec<&'a super::snapshot::Tab> {
    let Some(source) = snapshot.pane(pane) else {
        return Vec::new();
    };
    snapshot
        .tabs
        .iter()
        .filter(|tab| tab.tab_id != source.tab_id)
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct Help {
    pub scroll: usize,
    pub query: TextEditor,
    pub search_focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavTarget {
    Machine(String),
    Workspace(String),
    Pane(String),
    /// A live terminal without a pane, shown privately without changing the layout.
    Terminal(String),
    Loose,
}

#[derive(Clone, Debug, Default)]
pub struct Navigator {
    pub query: TextEditor,
    pub search_focused: bool,
    pub selected: Option<NavTarget>,
    pub scroll: usize,
    pub filter: Option<Status>,
}

pub struct NavRow {
    pub depth: u8,
    pub label: String,
    pub meta: String,
    pub detail: String,
    pub agent: Option<String>,
    pub status: Option<Status>,
    pub current: bool,
    pub target: NavTarget,
}

/// Rows for the session navigator: machines (when several), spaces, then their panes.
pub fn navigator_rows(
    snapshot: &Snapshot,
    machines: &[String],
    machine: &str,
    cwd: &dyn Fn(&str) -> Option<String>,
    navigator: &Navigator,
) -> Vec<NavRow> {
    let query = navigator.query.trim().to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let text = |value: &str| {
        let value = value.to_lowercase();
        words.iter().all(|word| value.contains(word))
    };
    let filtering = navigator.filter.is_some() || !query.is_empty();
    let federated = machines.len() > 1;
    let depth_offset = u8::from(federated);
    let mut rows = vec![];
    if federated {
        for name in machines.iter().filter(|name| *name != machine) {
            let label = if name.is_empty() { "Local" } else { name };
            if !filtering || (navigator.filter.is_none() && text(label)) {
                rows.push(NavRow {
                    depth: 0,
                    label: label.into(),
                    meta: String::new(),
                    detail: String::new(),
                    agent: None,
                    status: None,
                    current: false,
                    target: NavTarget::Machine(name.clone()),
                });
            }
        }
        rows.push(NavRow {
            depth: 0,
            label: if machine.is_empty() { "Local" } else { machine }.into(),
            meta: String::new(),
            detail: String::new(),
            agent: None,
            status: None,
            current: false,
            target: NavTarget::Machine(machine.into()),
        });
    }
    for workspace in &snapshot.workspaces {
        let workspace_matches =
            text(&workspace.label) || workspace.branch.as_deref().is_some_and(text);
        let tabs: Vec<_> = snapshot
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == workspace.workspace_id)
            .collect();
        let mut children = vec![];
        for tab in &tabs {
            let tab_matches = workspace_matches || text(&tab.label);
            let panes: Vec<_> = snapshot
                .panes
                .iter()
                .filter(|pane| pane.tab_id == tab.tab_id)
                .collect();
            for (index, pane) in panes.iter().enumerate() {
                let agent = snapshot.agents.iter().find(|a| a.pane_id == pane.pane_id);
                let status = agent.map_or(Status::Unknown, |a| a.agent_status);
                let agent_kind = agent.and_then(|a| a.agent.as_deref());
                let name = pane.label.as_deref();
                let title = agent.and_then(|a| a.title.as_deref());
                let tab_name = tab.custom_label.then_some(tab.label.as_str());
                let label = if panes.len() == 1 {
                    match name.or(tab_name).or(title) {
                        Some(label) => label.to_owned(),
                        None if tabs.len() > 1 => {
                            format!("{} · {}", agent_kind.unwrap_or("terminal"), tab.label)
                        }
                        None => workspace.label.clone(),
                    }
                } else {
                    let pane_name = name.or(title).or(agent_kind).unwrap_or("terminal");
                    match tab_name {
                        Some(tab_name) if tab_name != pane_name => {
                            format!("{tab_name} · {pane_name} · {}", index + 1)
                        }
                        _ => format!("{pane_name} · {}", index + 1),
                    }
                };
                let meta = pane.terminal.as_deref().and_then(cwd).unwrap_or_default();
                let filter = navigator.filter.is_none_or(|wanted| wanted == status);
                if filter
                    && (tab_matches
                        || text(&label)
                        || text(&meta)
                        || agent_kind.is_some_and(text)
                        || title.is_some_and(text)
                        || text(&pane.pane_id))
                {
                    children.push(NavRow {
                        depth: 1 + depth_offset,
                        label,
                        meta,
                        detail: format!("{} / {} / {}", workspace.label, tab.label, pane.pane_id),
                        agent: agent_kind.map(str::to_owned),
                        status: Some(status),
                        current: snapshot.focused_pane_id.as_deref() == Some(&pane.pane_id),
                        target: NavTarget::Pane(pane.pane_id.clone()),
                    });
                }
            }
        }
        if !filtering
            || !children.is_empty()
            || (navigator.filter.is_none() && !query.is_empty() && workspace_matches)
        {
            rows.push(NavRow {
                depth: depth_offset,
                label: workspace.label.clone(),
                meta: workspace.branch.clone().unwrap_or_default(),
                detail: workspace.cwd.clone(),
                agent: None,
                status: None,
                current: false,
                target: NavTarget::Workspace(workspace.workspace_id.clone()),
            });
            rows.extend(children);
        }
    }
    let loose: Vec<NavRow> = snapshot
        .loose
        .iter()
        .filter(|loose| {
            navigator
                .filter
                .is_none_or(|wanted| wanted == loose.agent_status)
        })
        .filter(|loose| text(&loose.label) || text(&loose.address) || text(&loose.cwd))
        .map(|loose| NavRow {
            depth: 1 + depth_offset,
            label: format!("{} · {}", loose.label, loose.address),
            meta: loose.cwd.clone(),
            detail: format!("no pane / {}", loose.address),
            agent: loose.reported.then(|| loose.label.clone()),
            status: Some(loose.agent_status),
            current: loose.focused,
            target: NavTarget::Terminal(loose.address.clone()),
        })
        .collect();
    if !loose.is_empty() {
        rows.push(NavRow {
            depth: depth_offset,
            label: "no pane".into(),
            meta: String::new(),
            detail: "live terminals that no pane shows".into(),
            agent: None,
            status: None,
            current: false,
            target: NavTarget::Loose,
        });
        rows.extend(loose);
    }
    rows
}

pub fn navigator_selected_index(rows: &[NavRow], navigator: &Navigator) -> Option<usize> {
    match &navigator.selected {
        Some(target) => rows.iter().position(|row| row.target == *target),
        None => rows
            .iter()
            .position(|row| matches!(row.target, NavTarget::Pane(_) | NavTarget::Terminal(_)))
            .or_else(|| (!rows.is_empty()).then_some(0)),
    }
}

#[derive(Clone, Debug)]
pub struct WorktreeCreate {
    pub repository: String,
    pub branch: TextEditor,
    pub new_branch: bool,
    pub checkout_path: TextEditor,
    pub path_focused: bool,
    pub custom_path: bool,
    pub creating: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct WorktreeEntry {
    pub path: String,
    pub branch: Option<String>,
    pub is_linked_worktree: bool,
    pub open_workspace_id: Option<String>,
    pub label: String,
}

impl WorktreeEntry {
    pub fn status_label(&self) -> &'static str {
        if self.open_workspace_id.is_some() {
            "open"
        } else if self.branch.is_some() {
            ""
        } else if self.is_linked_worktree {
            "detached"
        } else {
            "root"
        }
    }
    fn matches_query(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || format!(
                "{} {} {} {}",
                self.label,
                self.branch.as_deref().unwrap_or_default(),
                self.path,
                self.status_label()
            )
            .to_lowercase()
            .contains(&query)
    }
}

#[derive(Clone, Debug)]
pub struct WorktreeOpen {
    pub entries: Vec<WorktreeEntry>,
    pub selected: usize,
    pub query: TextEditor,
    pub search_focused: bool,
    pub error: Option<String>,
    pub opening: bool,
}

impl WorktreeOpen {
    pub fn filtered_indices(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.matches_query(&self.query).then_some(index))
            .collect()
    }
    pub fn selected_entry_index(&self) -> Option<usize> {
        let filtered = self.filtered_indices();
        filtered
            .contains(&self.selected)
            .then_some(self.selected)
            .or_else(|| filtered.first().copied())
    }
}

#[derive(Clone, Debug)]
pub struct WorktreeRemove {
    pub workspace_id: String,
    pub repository: String,
    pub path: String,
    pub error: Option<String>,
    pub removing: bool,
    pub review: Option<Value>,
    /// Herdr's second step: the reviewed checkout has work removal deletes.
    pub force_confirmation: bool,
}

#[derive(Clone, Debug)]
pub enum Overlay {
    GlobalMenu { highlighted: usize },
    ContextMenu(ContextMenu),
    Rename(Rename),
    Confirm(Confirm),
    PaneMove(PaneMove),
    Help(Help),
    Navigator(Navigator),
    WorktreeCreate(WorktreeCreate),
    WorktreeOpen(WorktreeOpen),
    WorktreeRemove(WorktreeRemove),
    Settings(Settings),
}

/// Herdr's settings sections that apply to this client; agent integrations belong to core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsSection {
    Theme,
    Indicators,
    Sound,
}

impl SettingsSection {
    pub const ALL: &[Self] = &[Self::Theme, Self::Indicators, Self::Sound];

    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::Indicators => "indicators",
            Self::Sound => "sound",
        }
    }

    pub fn choices(self) -> usize {
        match self {
            Self::Theme => crate::shell::palette::THEME_NAMES.len(),
            Self::Indicators | Self::Sound => 2,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub section: SettingsSection,
    pub selected: usize,
    /// The theme in effect when settings opened; Esc restores it after previews.
    pub original_theme: String,
}

impl Overlay {
    /// Menus float over live chrome; dialogs dim everything behind them.
    pub fn dims_background(&self) -> bool {
        !matches!(
            self,
            Self::GlobalMenu { .. } | Self::ContextMenu(_) | Self::Navigator(_)
        )
    }
}

/// Where an overlay drew its interactive parts, for mouse hit tests.
#[derive(Default)]
pub struct OverlayRender {
    pub area: Rect,
    pub menu_rows: Vec<(Rect, usize)>,
    pub primary: Rect,
    pub clear: Rect,
    pub cancel: Rect,
    pub navigator_rows: Vec<(Rect, NavTarget)>,
    pub navigator_search: Rect,
    pub worktree_search: Rect,
    pub worktree_rows: Vec<(Rect, usize)>,
    pub worktree_new_branch: Rect,
    pub worktree_existing_branch: Rect,
    pub worktree_branch_input: Rect,
    pub worktree_path_input: Rect,
    pub pane_move_rows: Vec<(Rect, String)>,
    pub settings_tabs: Vec<(Rect, SettingsSection)>,
    pub settings_choices: Vec<(Rect, usize)>,
    pub help_max_scroll: usize,
    pub cursor: Option<Position>,
}

fn panel(
    b: &mut Buffer,
    a: Rect,
    c: ratatui::style::Color,
    bg: ratatui::style::Color,
) -> Option<Rect> {
    if a.width < 2 || a.height < 2 {
        return None;
    }
    let background = Style::default().bg(bg).remove_modifier(Modifier::DIM);
    let border = Style::default().fg(c).bg(bg).remove_modifier(Modifier::DIM);
    for y in a.y..a.bottom() {
        for x in a.x..a.right() {
            b[(x, y)].set_symbol(" ").set_style(background);
        }
    }
    for x in a.x..a.right() {
        let (top, bottom) = if x == a.x {
            ("┌", "└")
        } else if x + 1 == a.right() {
            ("┐", "┘")
        } else {
            ("─", "─")
        };
        b[(x, a.y)].set_symbol(top).set_style(border);
        b[(x, a.bottom() - 1)].set_symbol(bottom).set_style(border);
    }
    for y in a.y + 1..a.bottom() - 1 {
        b[(a.x, y)].set_symbol("│").set_style(border);
        b[(a.right() - 1, y)].set_symbol("│").set_style(border);
    }
    Some(Rect::new(a.x + 1, a.y + 1, a.width - 2, a.height - 2))
}

fn popup(a: Rect, w: u16, h: u16) -> Option<Rect> {
    let w = w.min(a.width.saturating_sub(4));
    let h = h.min(a.height.saturating_sub(2));
    if w < 4 || h < 4 {
        return None;
    }
    Some(Rect::new(
        a.x + (a.width - w) / 2,
        a.y + (a.height - h) / 2,
        w,
        h,
    ))
}

fn button(b: &mut Buffer, r: Rect, t: &str, s: Style) {
    b.set_style(r, s);
    let w = display_width(t).min(r.width);
    put_text(b, r.x + (r.width - w) / 2, r.y, w, t, s)
}

fn row(i: Rect, ws: &[u16], gap: u16, off: u16) -> Vec<Rect> {
    let total = ws.iter().sum::<u16>() + gap * (ws.len().saturating_sub(1) as u16);
    let mut x = i.x + i.width.saturating_sub(total) / 2;
    ws.iter()
        .map(|w| {
            let r = Rect::new(
                x,
                i.y + off.min(i.height.saturating_sub(1)),
                (*w).min(i.width.saturating_sub(x - i.x)),
                1,
            );
            x += *w + gap;
            r
        })
        .collect()
}

/// Draws an editor's visible text and returns where its cursor belongs.
fn render_editor(
    b: &mut Buffer,
    area: Rect,
    editor: &TextEditor,
    style: Style,
) -> Option<Position> {
    if area.is_empty() {
        return None;
    }
    let (text, cursor) = editor.viewport(area.width);
    put_text(b, area.x, area.y, area.width, text, style);
    Some(Position::new(area.x + cursor.min(area.width - 1), area.y))
}

fn primary_style(p: &Palette) -> Style {
    Style::default()
        .fg(p.contrast())
        .bg(p.accent)
        .add_modifier(Modifier::BOLD)
}

fn secondary_style(p: &Palette) -> Style {
    Style::default()
        .fg(p.text)
        .bg(p.surface0)
        .add_modifier(Modifier::BOLD)
}

fn menu(
    b: &mut Buffer,
    rect: Rect,
    labels: &[&str],
    highlighted: usize,
    indent: bool,
    p: &Palette,
) -> Option<OverlayRender> {
    let inner = panel(b, rect, p.accent, p.panel_bg)?;
    let mut rows = vec![];
    for (index, label) in labels.iter().enumerate() {
        let y = inner.y.saturating_add(index as u16);
        if y >= inner.bottom() {
            break;
        }
        let row = Rect::new(inner.x, y, inner.width, 1);
        let style = if index == highlighted {
            primary_style(p)
        } else {
            Style::default().fg(p.text).bg(p.panel_bg)
        };
        b.set_style(row, style);
        let text = if indent {
            format!(" {label}")
        } else {
            (*label).to_owned()
        };
        put_text(b, row.x, row.y, row.width, &text, style);
        rows.push((row, index));
    }
    Some(OverlayRender {
        area: rect,
        menu_rows: rows,
        ..OverlayRender::default()
    })
}

pub fn render_global_menu(
    b: &mut Buffer,
    launcher: Rect,
    highlighted: usize,
    p: &Palette,
) -> Option<OverlayRender> {
    let items = global_menu_items();
    let screen = b.area;
    let width = items
        .iter()
        .map(|(label, _)| display_width(label))
        .max()
        .unwrap_or(8)
        .saturating_add(4)
        .min(screen.width.max(1));
    let height = (items.len() as u16)
        .saturating_add(2)
        .min(screen.height.max(1));
    let x = launcher
        .right()
        .saturating_sub(width)
        .min(screen.right().saturating_sub(width));
    let y = launcher.y.saturating_sub(height).max(screen.y);
    let labels: Vec<&str> = items.iter().map(|(label, _)| *label).collect();
    menu(
        b,
        Rect::new(x, y, width, height),
        &labels,
        highlighted,
        true,
        p,
    )
}

pub fn render_context_menu(b: &mut Buffer, m: &ContextMenu, p: &Palette) -> Option<OverlayRender> {
    let items = m.items();
    let screen = b.area;
    let width = items
        .iter()
        .map(|(label, _)| display_width(label))
        .max()
        .unwrap_or(0)
        .saturating_add(4)
        .max(14)
        .min(screen.width.max(1));
    let height = (items.len() as u16)
        .saturating_add(2)
        .min(screen.height.max(1));
    let x =
        m.x.min(screen.x.saturating_add(screen.width.saturating_sub(width)));
    let y = m.y.min(
        screen
            .y
            .saturating_add(screen.height.saturating_sub(height)),
    );
    let labels: Vec<&str> = items.iter().map(|(label, _)| *label).collect();
    menu(
        b,
        Rect::new(x, y, width, height),
        &labels,
        m.highlighted,
        false,
        p,
    )
}

pub struct Context<'a> {
    pub snapshot: &'a Snapshot,
    pub keys: &'a Keymap,
    pub palette: &'a Palette,
    pub symbols: bool,
    pub machines: &'a [String],
    pub machine: &'a str,
    pub cwd: &'a dyn Fn(&str) -> Option<String>,
}

pub fn render(b: &mut Buffer, overlay: &Overlay, cx: &Context<'_>) -> Option<OverlayRender> {
    if overlay.dims_background() {
        for y in b.area.y..b.area.bottom() {
            for x in b.area.x..b.area.right() {
                let c = &mut b[(x, y)];
                c.set_style(c.style().add_modifier(Modifier::DIM));
            }
        }
    }
    let p = cx.palette;
    match overlay {
        Overlay::Rename(v) => render_rename(b, v, p),
        Overlay::Confirm(v) => render_confirm(b, v, p),
        Overlay::PaneMove(v) => render_pane_move(b, v, cx.snapshot, p),
        Overlay::Help(v) => render_help(b, v, cx.keys, p),
        Overlay::Navigator(v) => render_navigator(b, v, cx),
        Overlay::WorktreeCreate(v) => render_worktree_create(b, v, p),
        Overlay::WorktreeOpen(v) => render_worktree_open(b, v, p),
        Overlay::WorktreeRemove(v) => render_worktree_remove(b, v, p),
        Overlay::Settings(v) => render_settings(b, v, p),
        Overlay::ContextMenu(v) => render_context_menu(b, v, p),
        Overlay::GlobalMenu { .. } => None,
    }
}

fn title_style(p: &Palette) -> Style {
    Style::default()
        .fg(p.text)
        .bg(p.panel_bg)
        .add_modifier(Modifier::BOLD)
}

fn render_pane_move(
    b: &mut Buffer,
    v: &PaneMove,
    snapshot: &Snapshot,
    p: &Palette,
) -> Option<OverlayRender> {
    let tabs = pane_move_destinations(snapshot, &v.pane_id);
    let height = (tabs.len() + 8).clamp(10, 24) as u16;
    let q = popup(b.area, 72, height)?;
    let i = panel(b, q, p.accent, p.panel_bg)?;
    put_text(b, i.x, i.y, i.width, "move pane to tab", title_style(p));
    let body = Rect::new(i.x, i.y + 2, i.width, i.height.saturating_sub(6));
    let selected = tabs
        .iter()
        .position(|tab| tab.tab_id == v.destination)
        .unwrap_or(0);
    let start = selected.saturating_sub(usize::from(body.height.saturating_sub(1)));
    let mut hits = Vec::new();
    for (index, tab) in tabs
        .iter()
        .skip(start)
        .take(usize::from(body.height))
        .enumerate()
    {
        let rect = Rect::new(body.x, body.y + index as u16, body.width, 1);
        let style = if tab.tab_id == v.destination {
            primary_style(p)
        } else {
            Style::default().fg(p.text).bg(p.panel_bg)
        };
        b.set_style(rect, style);
        let workspace = snapshot
            .workspace(&tab.workspace_id)
            .map_or("", |w| w.label.as_str());
        put_text(
            b,
            rect.x,
            rect.y,
            rect.width,
            &format!(" {workspace} / {}", tab.label),
            style,
        );
        hits.push((rect, tab.tab_id.clone()));
    }
    if tabs.is_empty() {
        put_text(
            b,
            body.x,
            body.y,
            body.width,
            " no other tabs; create a tab or workspace first",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
    status_line(b, i, i.bottom() - 3, false, "", v.error.as_deref(), p);
    put_text(
        b,
        i.x,
        i.bottom() - 2,
        i.width,
        " ↑↓/j/k choose destination",
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    let buttons = row(i, &[10, 12], 2, i.height.saturating_sub(1));
    let [primary, cancel] = buttons.as_slice() else {
        return None;
    };
    button(b, *primary, " ↵ move ", primary_style(p));
    button(b, *cancel, " esc cancel ", secondary_style(p));
    Some(OverlayRender {
        area: q,
        primary: *primary,
        cancel: *cancel,
        pane_move_rows: hits,
        ..OverlayRender::default()
    })
}

fn render_rename(b: &mut Buffer, v: &Rename, p: &Palette) -> Option<OverlayRender> {
    let q = popup(b.area, 56, 7)?;
    let i = panel(b, q, p.accent, p.panel_bg)?;
    put_text(b, i.x, i.y, i.width, v.title, title_style(p));
    let input = Rect::new(i.x, i.y + 2, i.width, 1);
    b.set_style(input, Style::default().fg(p.text).bg(p.surface0));
    let cursor = render_editor(
        b,
        Rect::new(input.x + 1, input.y, input.width.saturating_sub(1), 1),
        &v.input,
        Style::default().fg(p.text).bg(p.surface0),
    );
    let rs = row(i, &[8, 10, 12], 2, 3);
    let [save, clear, cancel] = rs.as_slice() else {
        return None;
    };
    button(b, *save, " ↵ save ", primary_style(p));
    button(b, *clear, " ^c clear ", secondary_style(p));
    button(b, *cancel, " esc cancel ", secondary_style(p));
    Some(OverlayRender {
        area: q,
        primary: *save,
        clear: *clear,
        cancel: *cancel,
        cursor,
        ..OverlayRender::default()
    })
}

fn render_confirm(b: &mut Buffer, c: &Confirm, p: &Palette) -> Option<OverlayRender> {
    let q = popup(b.area, 64, 6)?;
    let i = panel(b, q, p.red, p.panel_bg)?;
    put_text(
        b,
        i.x,
        i.y,
        i.width,
        &format!(" {}", c.title),
        Style::default()
            .fg(p.red)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        b,
        i.x,
        i.y + 1,
        i.width,
        &format!(" {}", c.detail),
        Style::default().fg(p.text).bg(p.panel_bg),
    );
    let rs = row(i, &[13, 12], 2, 3);
    let [ok, cancel] = rs.as_slice() else {
        return None;
    };
    button(
        b,
        *ok,
        " ↵ confirm ",
        Style::default()
            .fg(p.contrast())
            .bg(p.red)
            .add_modifier(Modifier::BOLD),
    );
    button(b, *cancel, " esc cancel ", secondary_style(p));
    Some(OverlayRender {
        area: q,
        primary: *ok,
        cancel: *cancel,
        ..OverlayRender::default()
    })
}

fn help_lines(keys: &Keymap, query: &str, p: &Palette) -> Vec<(usize, Line<'static>)> {
    let groups = filter_help_groups(help_groups(keys), query);
    let key_width = groups
        .iter()
        .flat_map(|(_, entries)| entries.iter().map(|(key, _)| key.chars().count()))
        .max()
        .unwrap_or(8);
    if groups.is_empty() {
        let message = " no matching keybinds";
        return vec![(
            message.chars().count(),
            Line::from(Span::styled(
                message,
                Style::default().fg(p.overlay1).bg(p.panel_bg),
            )),
        )];
    }
    let mut lines = vec![];
    for (group, entries) in groups {
        lines.push((
            group.len() + 1,
            Line::from(Span::styled(
                format!(" {group}"),
                Style::default()
                    .fg(p.accent)
                    .bg(p.panel_bg)
                    .add_modifier(Modifier::BOLD),
            )),
        ));
        for (key, label) in entries {
            let padded = format!(" {key:<key_width$} ");
            lines.push((
                padded.chars().count() + label.chars().count(),
                Line::from(vec![
                    Span::styled(
                        padded,
                        Style::default()
                            .fg(p.mauve)
                            .bg(p.panel_bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(label, Style::default().fg(p.text).bg(p.panel_bg)),
                ]),
            ));
        }
        lines.push((0, Line::raw("")));
    }
    lines
}

fn render_help(b: &mut Buffer, h: &Help, keys: &Keymap, p: &Palette) -> Option<OverlayRender> {
    let q = popup(b.area, 76, 22)?;
    let i = panel(b, q, p.accent, p.panel_bg)?;
    if i.width < 20 || i.height < 6 {
        return None;
    }
    put_text(b, i.x, i.y, i.width, "keybinds", title_style(p));
    let close = Rect::new(i.right() - 13, i.y, 13, 1);
    button(
        b,
        close,
        if h.search_focused {
            " esc back "
        } else {
            " esc close "
        },
        primary_style(p),
    );
    let sy = i.y + 1;
    put_text(
        b,
        i.x,
        sy,
        i.width,
        if h.search_focused {
            " / "
        } else {
            " / press / to filter by command or shortcut"
        },
        Style::default()
            .fg(if h.search_focused { p.text } else { p.overlay0 })
            .bg(p.panel_bg),
    );
    let cursor = if h.search_focused {
        render_editor(
            b,
            Rect::new(i.x + 3, sy, i.width.saturating_sub(3), 1),
            &h.query,
            Style::default().fg(p.text).bg(p.panel_bg),
        )
    } else {
        None
    };
    let body = Rect::new(i.x, i.y + 3, i.width, i.height.saturating_sub(5));
    let lines = help_lines(keys, &h.query, p);
    let viewport_rows = usize::from(body.height.max(1));
    let wrapped_rows = |width: u16| {
        let width = usize::from(width.max(1));
        lines
            .iter()
            .map(|(line_width, _)| (*line_width).max(1).div_ceil(width))
            .sum::<usize>()
    };
    let needs_scrollbar = wrapped_rows(body.width) > viewport_rows;
    let text_area = if needs_scrollbar {
        Rect::new(body.x, body.y, body.width.saturating_sub(1), body.height)
    } else {
        body
    };
    let max_scroll = wrapped_rows(text_area.width).saturating_sub(viewport_rows);
    let scroll = h.scroll.min(max_scroll);
    Paragraph::new(lines.into_iter().map(|(_, line)| line).collect::<Vec<_>>())
        .wrap(Wrap { trim: false })
        .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0))
        .render(text_area, b);
    if needs_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        let metrics = ScrollMetrics {
            offset_from_bottom: max_scroll.saturating_sub(scroll),
            max_offset_from_bottom: max_scroll,
            viewport_rows,
        };
        if let Some(thumb) = scrollbar_thumb(metrics, track) {
            for y in track.y..track.bottom() {
                b[(track.x, y)]
                    .set_symbol("▐")
                    .set_style(Style::default().fg(p.overlay0).bg(p.panel_bg));
            }
            for y in thumb.top..thumb.top.saturating_add(thumb.len) {
                b[(track.x, y)]
                    .set_symbol("▐")
                    .set_style(Style::default().fg(p.overlay1).bg(p.panel_bg));
            }
        }
    }
    put_text(
        b,
        i.x,
        i.bottom() - 1,
        i.width,
        if h.search_focused {
            " edit ←→/home/end · kill ^u/^k · yank ^y · scroll ↑↓ · back esc"
        } else {
            " search / · scroll j/k/↑↓/pgup/pgdn · close esc/enter"
        },
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    Some(OverlayRender {
        area: q,
        cancel: close,
        help_max_scroll: max_scroll,
        cursor,
        ..OverlayRender::default()
    })
}

fn render_navigator(b: &mut Buffer, n: &Navigator, cx: &Context<'_>) -> Option<OverlayRender> {
    let p = cx.palette;
    let a = b.area;
    let width = a.width.saturating_sub(4).min(116);
    let height = a.height.saturating_sub(2).min(42);
    if width < 4 || height < 9 {
        return None;
    }
    let q = Rect::new(
        a.x + (a.width - width) / 2,
        a.y + (a.height - height) / 2,
        width,
        height,
    );
    let i = panel(b, q, p.accent, p.panel_bg)?;
    put_text(
        b,
        q.x + 2,
        q.y,
        q.width.saturating_sub(4),
        " Go to ",
        Style::default().fg(p.accent).bg(p.panel_bg),
    );
    let rows = navigator_rows(cx.snapshot, cx.machines, cx.machine, cx.cwd, n);
    let search = if n.search_focused {
        " / ".to_owned()
    } else if let Some(status) = n.filter {
        format!(" / {}", status.text())
    } else if n.query.is_empty() {
        " / search agents and terminals".to_owned()
    } else {
        format!(" / {}", n.query.as_str())
    };
    let terminal_count = rows
        .iter()
        .filter(|row| matches!(row.target, NavTarget::Pane(_) | NavTarget::Terminal(_)))
        .count();
    let count = format!(
        "{terminal_count} {}",
        if terminal_count == 1 {
            "terminal"
        } else {
            "terminals"
        }
    );
    put_text(
        b,
        i.x,
        i.y,
        i.width.saturating_sub(display_width(&count) + 1),
        &search,
        Style::default()
            .fg(if n.search_focused { p.text } else { p.overlay0 })
            .bg(p.panel_bg),
    );
    let cursor = if n.search_focused {
        render_editor(
            b,
            Rect::new(
                i.x + 3,
                i.y,
                i.width.saturating_sub(4 + display_width(&count)),
                1,
            ),
            &n.query,
            Style::default().fg(p.text).bg(p.panel_bg),
        )
    } else {
        None
    };
    put_right_text(
        b,
        i,
        i.y,
        &count,
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    put_text(
        b,
        i.x,
        i.y + 1,
        i.width,
        &"─".repeat(i.width as usize),
        Style::default().fg(p.surface1).bg(p.panel_bg),
    );
    let body = Rect::new(i.x, i.y + 2, i.width, i.height.saturating_sub(5));
    let selected = navigator_selected_index(&rows, n).unwrap_or(0);
    let max = rows.len().saturating_sub(body.height as usize);
    let scroll = n
        .scroll
        .max(selected.saturating_sub(body.height.saturating_sub(1) as usize))
        .min(selected)
        .min(max);
    let scrollbar =
        (max > 0 && body.width > 1).then(|| Rect::new(body.right() - 1, body.y, 1, body.height));
    let row_width = body.width.saturating_sub(u16::from(scrollbar.is_some()));
    let mut row_hits = vec![];
    if rows.is_empty() {
        put_text(
            b,
            body.x,
            body.y,
            body.width,
            " No matching agents or terminals",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
    for (ix, r) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(body.height as usize)
    {
        let rect = Rect::new(body.x, body.y + (ix - scroll) as u16, row_width, 1);
        row_hits.push((rect, r.target.clone()));
        let st = if ix == selected {
            primary_style(p)
        } else {
            Style::default()
                .fg(if matches!(r.target, NavTarget::Machine(_)) {
                    p.subtext0
                } else {
                    p.text
                })
                .bg(p.panel_bg)
        };
        let is_pane = matches!(r.target, NavTarget::Pane(_) | NavTarget::Terminal(_));
        let connector = if !is_pane {
            ""
        } else if rows
            .get(ix + 1)
            .is_some_and(|next| matches!(next.target, NavTarget::Pane(_) | NavTarget::Terminal(_)))
        {
            "├─ "
        } else {
            "└─ "
        };
        let padding = u16::from(r.depth.saturating_sub(u8::from(is_pane))) * 2 + 1;
        let connector_x = rect.x + padding;
        let indent = format!("{:width$}{connector}", "", width = usize::from(padding));
        let current = if r.current { "◆ " } else { "" };
        let status = r
            .status
            .map(|s| status_icon(s, cx.symbols))
            .unwrap_or_default();
        let status_separator = if status.is_empty() { "" } else { " " };
        let label = format!("{indent}{current}{status}{status_separator}{}", r.label);
        let st = if r.status.is_none() {
            st.add_modifier(Modifier::BOLD)
        } else {
            st
        };
        b.set_style(rect, st);
        let columns = if r.status.is_some() {
            if rect.width >= 64 {
                24
            } else if rect.width >= 36 {
                12
            } else {
                0
            }
        } else {
            0
        };
        put_text(
            b,
            rect.x,
            rect.y,
            rect.width.saturating_sub(columns),
            &label,
            st,
        );
        if is_pane {
            put_text(
                b,
                connector_x,
                rect.y,
                rect.right().saturating_sub(connector_x).min(2),
                connector,
                if ix == selected {
                    st
                } else {
                    st.fg(p.overlay0)
                },
            );
        }
        if let Some(status) = r.status {
            let prefix = format!("{indent}{current}");
            let icon = status_icon(status, cx.symbols);
            put_text(
                b,
                rect.x.saturating_add(display_width(&prefix)),
                rect.y,
                display_width(icon),
                icon,
                if ix == selected {
                    st
                } else {
                    Style::default().fg(status_color(status, p)).bg(p.panel_bg)
                },
            );
            let meta_style = if ix == selected {
                st
            } else {
                st.fg(p.overlay0)
            };
            if columns > 0 {
                put_text(
                    b,
                    rect.right() - columns + 1,
                    rect.y,
                    11,
                    r.agent.as_deref().unwrap_or("terminal"),
                    meta_style,
                );
            }
            if columns == 24 {
                put_text(
                    b,
                    rect.right() - 11,
                    rect.y,
                    11,
                    if r.agent.is_some() {
                        status.text()
                    } else {
                        "shell"
                    },
                    meta_style,
                );
            }
        } else if !r.meta.is_empty() {
            let label_width = display_width(&label).min(rect.width);
            let meta = Rect::new(
                rect.x.saturating_add(label_width).saturating_add(1),
                rect.y,
                rect.width.saturating_sub(label_width.saturating_add(1)),
                1,
            );
            put_right_text(b, meta, rect.y, &r.meta, st);
        }
    }
    if let Some(track) = scrollbar {
        let metrics = ScrollMetrics {
            offset_from_bottom: max.saturating_sub(scroll),
            max_offset_from_bottom: max,
            viewport_rows: usize::from(body.height),
        };
        if let Some(thumb) = scrollbar_thumb(metrics, track) {
            for y in track.y..track.bottom() {
                b[(track.x, y)]
                    .set_symbol("▕")
                    .set_style(Style::default().fg(p.overlay0));
            }
            for y in thumb.top..thumb.top + thumb.len {
                b[(track.x, y)]
                    .set_symbol("▐")
                    .set_style(Style::default().fg(p.overlay1));
            }
        }
    }
    if let Some(r) = rows.get(selected) {
        put_text(
            b,
            i.x,
            i.bottom() - 3,
            i.width,
            &format!(" {}", r.detail),
            Style::default().fg(p.subtext0).bg(p.panel_bg),
        );
        put_text(
            b,
            i.x,
            i.bottom() - 2,
            i.width,
            &format!(" {}", r.meta),
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
    put_text(
        b,
        i.x,
        i.bottom() - 1,
        i.width,
        if n.search_focused {
            " search type · move ↑↓/ctrl+n/p · open enter · back esc"
        } else {
            " ↑↓/j/k rows · ←→ workspace · / search · a/b/w/i/d filter · enter open · esc close"
        },
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    Some(OverlayRender {
        area: q,
        navigator_search: Rect::new(i.x, i.y, i.width, 1),
        navigator_rows: row_hits,
        cursor,
        ..OverlayRender::default()
    })
}

fn render_worktree_create(
    b: &mut Buffer,
    c: &WorktreeCreate,
    p: &Palette,
) -> Option<OverlayRender> {
    let q = popup(b.area, 68, 12)?;
    let i = panel(b, q, p.accent, p.panel_bg)?;
    put_text(b, i.x, i.y, i.width, "new worktree", title_style(p));
    let modes = row(i, &[14, 19], 2, 1);
    let [new_branch, existing_branch] = modes.as_slice() else {
        return None;
    };
    button(
        b,
        *new_branch,
        " new branch ",
        if c.new_branch {
            primary_style(p)
        } else {
            secondary_style(p)
        },
    );
    button(
        b,
        *existing_branch,
        " existing branch ",
        if c.new_branch {
            secondary_style(p)
        } else {
            primary_style(p)
        },
    );
    let label = Style::default().fg(p.overlay0).bg(p.panel_bg);
    put_text(
        b,
        i.x,
        i.y + 2,
        i.width,
        " branch name · tab new/existing · ↑↓ branch/path",
        label,
    );
    let input = Rect::new(i.x, i.y + 3, i.width, 1);
    b.set_style(input, Style::default().fg(p.text).bg(p.surface0));
    let branch_cursor = render_editor(
        b,
        Rect::new(input.x + 1, input.y, input.width.saturating_sub(1), 1),
        &c.branch,
        Style::default().fg(p.text).bg(p.surface0),
    );
    put_text(b, i.x, i.y + 5, i.width, " checkout path", label);
    let path_input = Rect::new(i.x, i.y + 6, i.width, 1);
    b.set_style(path_input, Style::default().fg(p.text).bg(p.surface0));
    let path_cursor = render_editor(
        b,
        Rect::new(
            path_input.x + 1,
            path_input.y,
            path_input.width.saturating_sub(1),
            1,
        ),
        &c.checkout_path,
        Style::default().fg(p.text).bg(p.surface0),
    );
    let cursor = if c.path_focused {
        path_cursor
    } else {
        branch_cursor
    };
    status_line(
        b,
        i,
        i.y + 8,
        c.creating,
        " creating…",
        c.error.as_deref(),
        p,
    );
    let rs = row(i, &[20, 12], 2, 9);
    let [primary, cancel] = rs.as_slice() else {
        return None;
    };
    button(b, *primary, " ↵ create and open ", primary_style(p));
    button(b, *cancel, " esc cancel ", secondary_style(p));
    Some(OverlayRender {
        area: q,
        primary: *primary,
        cancel: *cancel,
        worktree_new_branch: *new_branch,
        worktree_existing_branch: *existing_branch,
        worktree_branch_input: input,
        worktree_path_input: path_input,
        cursor: cursor.filter(|_| !c.creating),
        ..OverlayRender::default()
    })
}

fn status_line(
    b: &mut Buffer,
    i: Rect,
    y: u16,
    busy: bool,
    busy_text: &str,
    error: Option<&str>,
    p: &Palette,
) {
    if busy {
        put_text(
            b,
            i.x,
            y,
            i.width,
            busy_text,
            Style::default().fg(p.accent).bg(p.panel_bg),
        );
    } else if let Some(error) = error {
        put_text(
            b,
            i.x,
            y,
            i.width,
            &format!(" {error}"),
            Style::default().fg(p.red).bg(p.panel_bg),
        );
    }
}

fn render_worktree_open(b: &mut Buffer, o: &WorktreeOpen, p: &Palette) -> Option<OverlayRender> {
    let height = (o.entries.len().saturating_mul(2) + 7).clamp(12, 26) as u16;
    let q = popup(b.area, 96, height)?;
    let i = panel(b, q, p.accent, p.panel_bg)?;
    put_text(b, i.x, i.y, i.width, "open worktree", title_style(p));
    let search = Rect::new(i.x, i.y + 1, i.width, 1);
    let filtered = o.filtered_indices();
    put_text(
        b,
        search.x,
        search.y,
        search.width,
        &if o.search_focused {
            " / ".to_owned()
        } else if !o.query.is_empty() {
            format!(" / {}", o.query.as_str())
        } else {
            " / filter worktrees".to_owned()
        },
        Style::default()
            .fg(if o.search_focused { p.text } else { p.overlay0 })
            .bg(p.panel_bg),
    );
    let count = if filtered.len() == o.entries.len() {
        format!("{} checkouts", o.entries.len())
    } else {
        format!("{}/{} checkouts", filtered.len(), o.entries.len())
    };
    let cursor = if o.search_focused {
        render_editor(
            b,
            Rect::new(
                search.x + 3,
                search.y,
                search.width.saturating_sub(4 + display_width(&count)),
                1,
            ),
            &o.query,
            Style::default().fg(p.text).bg(p.panel_bg),
        )
    } else {
        None
    };
    put_right_text(
        b,
        search,
        search.y,
        &count,
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    put_text(
        b,
        i.x,
        i.y + 2,
        i.width,
        &"─".repeat(i.width as usize),
        Style::default().fg(p.surface1).bg(p.panel_bg),
    );
    let body = Rect::new(i.x, i.y + 3, i.width, i.height.saturating_sub(6));
    let visible_count = (body.height / 2).max(1) as usize;
    let selected_position = filtered
        .iter()
        .position(|index| *index == o.selected)
        .unwrap_or(0);
    let start = selected_position
        .saturating_sub(visible_count.saturating_sub(1))
        .min(filtered.len().saturating_sub(visible_count));
    let mut row_hits = vec![];
    for (visible, entry_index) in filtered
        .iter()
        .copied()
        .skip(start)
        .take(visible_count)
        .enumerate()
    {
        let entry = &o.entries[entry_index];
        let rect = Rect::new(body.x, body.y + visible as u16 * 2, body.width, 2);
        row_hits.push((rect, entry_index));
        let selected = entry_index == o.selected;
        let style = if selected {
            Style::default().fg(p.contrast()).bg(p.accent)
        } else {
            Style::default().fg(p.text).bg(p.panel_bg)
        };
        b.set_style(rect, style);
        put_text(
            b,
            rect.x,
            rect.y,
            rect.width,
            &format!(" {}", entry.label),
            style.add_modifier(Modifier::BOLD),
        );
        let status = entry.status_label();
        if !status.is_empty() {
            put_right_text(b, rect, rect.y, status, style);
        }
        put_text(
            b,
            rect.x,
            rect.y + 1,
            rect.width,
            &format!(" {}", entry.path),
            if selected {
                style
            } else {
                Style::default().fg(p.overlay0).bg(p.panel_bg)
            },
        );
    }
    if filtered.is_empty() {
        put_text(
            b,
            body.x,
            body.y,
            body.width,
            " no matching worktrees",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
    status_line(
        b,
        i,
        i.bottom() - 3,
        o.opening,
        " opening…",
        o.error.as_deref(),
        p,
    );
    let rs = row(i, &[10, 12], 2, i.height.saturating_sub(1));
    let [primary, cancel] = rs.as_slice() else {
        return None;
    };
    button(b, *primary, " ↵ open ", primary_style(p));
    button(b, *cancel, " esc cancel ", secondary_style(p));
    Some(OverlayRender {
        area: q,
        primary: *primary,
        cancel: *cancel,
        worktree_search: search,
        worktree_rows: row_hits,
        cursor: cursor.filter(|_| !o.opening),
        ..OverlayRender::default()
    })
}

fn render_worktree_remove(
    b: &mut Buffer,
    r: &WorktreeRemove,
    p: &Palette,
) -> Option<OverlayRender> {
    let q = popup(b.area, 72, 10)?;
    let i = panel(b, q, p.red, p.panel_bg)?;
    put_text(
        b,
        i.x,
        i.y,
        i.width,
        " delete worktree checkout?",
        Style::default()
            .fg(p.red)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    let text = Style::default().fg(p.text).bg(p.panel_bg);
    put_text(
        b,
        i.x,
        i.y + 1,
        i.width,
        " This removes the checkout folder:",
        text,
    );
    put_text(
        b,
        i.x,
        i.y + 2,
        i.width,
        &format!(" {}", r.path),
        Style::default().fg(p.subtext0).bg(p.panel_bg),
    );
    put_text(
        b,
        i.x,
        i.y + 3,
        i.width,
        " The branch is not deleted. The workspace will close.",
        text,
    );
    if r.force_confirmation {
        put_text(
            b,
            i.x,
            i.y + 4,
            i.width,
            " Dirty or untracked files will be permanently deleted.",
            Style::default().fg(p.red).bg(p.panel_bg),
        );
    }
    status_line(
        b,
        i,
        i.y + 5,
        r.removing,
        " removing…",
        r.error.as_deref(),
        p,
    );
    let rs = row(i, &[18, 12], 2, 7);
    let [primary, cancel] = rs.as_slice() else {
        return None;
    };
    button(
        b,
        *primary,
        if r.force_confirmation {
            " ↵ delete anyway "
        } else {
            " ↵ remove "
        },
        Style::default()
            .fg(p.contrast())
            .bg(p.red)
            .add_modifier(Modifier::BOLD),
    );
    button(b, *cancel, " esc cancel ", secondary_style(p));
    Some(OverlayRender {
        area: q,
        primary: *primary,
        cancel: *cancel,
        ..OverlayRender::default()
    })
}

// Settings, adapted from Herdr `src/client/shell/settings_overlay.rs`.

fn choice_style(selected: bool, p: &Palette) -> Style {
    if selected {
        Style::default()
            .fg(p.contrast())
            .bg(p.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(p.text).bg(p.panel_bg)
    }
}

fn draw_choice(
    b: &mut Buffer,
    rect: Rect,
    label: &str,
    selected: bool,
    current: bool,
    p: &Palette,
) {
    let style = choice_style(selected, p);
    b.set_style(rect, style);
    let marker = if selected { "▸" } else { " " };
    let current = if current { " ✓" } else { "" };
    put_text(
        b,
        rect.x,
        rect.y,
        rect.width,
        &format!(" {marker} {label}{current}"),
        style,
    );
}

fn render_settings(b: &mut Buffer, settings: &Settings, p: &Palette) -> Option<OverlayRender> {
    let q = popup(b.area, 76, 22)?;
    let inner = panel(b, q, p.accent, p.panel_bg)?;
    if inner.width < 20 || inner.height < 8 {
        return None;
    }
    put_text(
        b,
        inner.x,
        inner.y,
        inner.width,
        " settings",
        Style::default()
            .fg(p.text)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    let mut tab_x = inner.x;
    let mut tab_hits = Vec::new();
    for section in SettingsSection::ALL {
        let label = format!(" {} ", section.label());
        let width = display_width(&label).min(inner.right().saturating_sub(tab_x));
        let rect = Rect::new(tab_x, inner.y + 1, width, 1);
        let style = if *section == settings.section {
            Style::default()
                .fg(p.contrast())
                .bg(p.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.overlay1).bg(p.panel_bg)
        };
        b.set_style(rect, style);
        put_text(b, rect.x, rect.y, rect.width, &label, style);
        tab_hits.push((rect, *section));
        tab_x = tab_x.saturating_add(width.saturating_add(1));
        if tab_x >= inner.right() {
            break;
        }
    }
    put_text(
        b,
        inner.x,
        inner.y + 2,
        inner.width,
        &"─".repeat(inner.width as usize),
        Style::default().fg(p.surface0).bg(p.panel_bg),
    );
    let content = Rect::new(
        inner.x,
        inner.y + 4,
        inner.width,
        inner.height.saturating_sub(7),
    );
    let mut choice_hits = Vec::new();
    match settings.section {
        SettingsSection::Theme => {
            let names = crate::shell::palette::THEME_NAMES;
            let visible = usize::from(content.height);
            let scroll = settings.selected.saturating_sub(visible.saturating_sub(1));
            for (visible_index, (index, name)) in names
                .iter()
                .enumerate()
                .skip(scroll)
                .take(visible)
                .enumerate()
            {
                let rect = Rect::new(
                    content.x,
                    content.y + visible_index as u16,
                    content.width,
                    1,
                );
                let current = crate::shell::palette::normalized_theme_name(name)
                    == crate::shell::palette::normalized_theme_name(&settings.original_theme);
                draw_choice(b, rect, name, index == settings.selected, current, p);
                choice_hits.push((rect, index));
            }
        }
        SettingsSection::Indicators => render_choice_section(
            b,
            content,
            "agent status indicators",
            "choose color dots or distinct symbols for each state",
            &["color dots  ● ● ● ○ ·", "distinct symbols  × ◐ ✓ ○ ·"],
            settings.selected,
            p,
            &mut choice_hits,
        ),
        SettingsSection::Sound => render_choice_section(
            b,
            content,
            "sound alerts",
            "play sounds when agents change state in background",
            &["on", "off"],
            settings.selected,
            p,
            &mut choice_hits,
        ),
    }
    let buttons = row(inner, &[10, 12], 2, inner.height.saturating_sub(1));
    let (primary, close) = (buttons[0], buttons[1]);
    button(
        b,
        primary,
        " ↵ apply ",
        Style::default()
            .fg(p.contrast())
            .bg(p.accent)
            .add_modifier(Modifier::BOLD),
    );
    button(
        b,
        close,
        " esc close ",
        Style::default()
            .fg(p.text)
            .bg(p.surface0)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        b,
        inner.x,
        inner.bottom().saturating_sub(2),
        inner.width,
        " ↑↓ select  tab section",
        Style::default().fg(p.overlay1).bg(p.panel_bg),
    );
    Some(OverlayRender {
        area: q,
        primary,
        cancel: close,
        settings_tabs: tab_hits,
        settings_choices: choice_hits,
        ..OverlayRender::default()
    })
}

#[allow(clippy::too_many_arguments)]
fn render_choice_section(
    b: &mut Buffer,
    area: Rect,
    title: &str,
    description: &str,
    choices: &[&str],
    selected: usize,
    p: &Palette,
    hits: &mut Vec<(Rect, usize)>,
) {
    put_text(
        b,
        area.x,
        area.y,
        area.width,
        title,
        Style::default()
            .fg(p.text)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        b,
        area.x,
        area.y + 1,
        area.width,
        description,
        Style::default().fg(p.overlay1).bg(p.panel_bg),
    );
    let row_gap = u16::from(choices.len() > 2);
    for (index, choice) in choices.iter().enumerate() {
        let y = area.y + 3 + index as u16 * (1 + row_gap);
        if y >= area.bottom() {
            break;
        }
        let rect = Rect::new(area.x, y, area.width, 1);
        draw_choice(b, rect, choice, index == selected, false, p);
        hits.push((rect, index));
    }
}

/// A transient message in the top-right corner, like Herdr's notification toasts.
pub fn render_toast(b: &mut Buffer, area: Rect, text: &str, error: bool, p: &Palette) -> Rect {
    let width = (display_width(text) + 4)
        .min(area.width.saturating_sub(2))
        .min(64);
    if width < 6 || area.height < 4 {
        return Rect::default();
    }
    let rect = Rect::new(area.right().saturating_sub(width + 1), area.y + 1, width, 3);
    let Some(inner) = panel(b, rect, if error { p.red } else { p.accent }, p.panel_bg) else {
        return Rect::default();
    };
    put_text(
        b,
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        text,
        Style::default().fg(p.text).bg(p.panel_bg),
    );
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::snapshot::{Agent, Pane, Tab, Workspace};

    fn snapshot() -> Snapshot {
        let workspace = |id: &str, label: &str| Workspace {
            workspace_id: id.into(),
            number: 1,
            label: label.into(),
            custom_label: false,
            cwd: format!("/src/{label}"),
            repository: None,
            branch: Some("main".into()),
            git_ahead_behind: None,
            worktree: None,
            focused: false,
            agent_status: Status::Unknown,
        };
        let tab = |id: &str, space: &str| Tab {
            tab_id: id.into(),
            workspace_id: space.into(),
            number: 1,
            label: "1".into(),
            custom_label: false,
            zoomed: false,
            focused: false,
            agent_status: Status::Unknown,
        };
        let pane = |id: &str, tab: &str, space: &str| Pane {
            pane_id: id.into(),
            workspace_id: space.into(),
            tab_id: tab.into(),
            label: None,
            terminal: Some(format!("t-{id}")),
            focused: false,
        };
        Snapshot {
            focused_pane_id: Some("p2".into()),
            workspaces: vec![workspace("s1", "api"), workspace("s2", "web")],
            tabs: vec![tab("t1", "s1"), tab("t2", "s2")],
            panes: vec![
                pane("p1", "t1", "s1"),
                pane("p2", "t2", "s2"),
                pane("p3", "t2", "s2"),
            ],
            agents: vec![Agent {
                pane_id: "p3".into(),
                address: "t-p3".into(),
                workspace_id: Some("s2".into()),
                tab_id: Some("t2".into()),
                workspace_label: "web".into(),
                agent: Some("claude".into()),
                title: None,
                agent_status: Status::Blocked,
                state_change_seq: 1,
                focused: false,
            }],
            ..Snapshot::default()
        }
    }

    #[test]
    fn navigator_lists_spaces_with_their_panes_and_filters_by_status() {
        let snapshot = snapshot();
        let none = |_: &str| None;
        let mut navigator = Navigator::default();
        let labels = |n: &Navigator| {
            navigator_rows(&snapshot, &[String::new()], "", &none, n)
                .into_iter()
                .map(|r| r.label)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            labels(&navigator),
            vec!["api", "api", "web", "terminal · 1", "claude · 2"]
        );
        let rows = navigator_rows(&snapshot, &[String::new()], "", &none, &navigator);
        assert_eq!(navigator_selected_index(&rows, &navigator), Some(1));
        assert!(rows[3].current);
        navigator.filter = Some(Status::Blocked);
        assert_eq!(labels(&navigator), vec!["web", "claude · 2"]);
        navigator.filter = None;
        navigator.query = TextEditor::new("api", false);
        assert_eq!(labels(&navigator), vec!["api", "api"]);
    }

    #[test]
    fn workspace_menus_follow_git_and_group_state() {
        let menu = |is_git, linked, children| ContextMenu {
            target: MenuTarget::Workspace {
                workspace_id: "s".into(),
                is_git,
                is_linked_worktree: linked,
                has_worktree_children: children,
                close_group: children,
                collapsed: false,
            },
            x: 0,
            y: 0,
            highlighted: 0,
        };
        let labels = |m: ContextMenu| m.items().into_iter().map(|(l, _)| l).collect::<Vec<_>>();
        assert_eq!(labels(menu(false, false, false)), vec!["Rename", "Close"]);
        assert_eq!(
            labels(menu(true, true, false)),
            vec!["Rename", "Close", "Delete worktree checkout..."]
        );
        assert_eq!(
            labels(menu(true, false, true)),
            vec![
                "Rename",
                "Close group",
                "New worktree",
                "Open worktree...",
                "Collapse"
            ]
        );
    }
}
