//! Sidebar, agents panel, tab bar, and mode bar.
//! Adapted from Herdr `src/client/shell/{sidebar,agent_sidebar,tabs,render,scroll}.rs`
//! and `src/ui/{sidebar,sidebar/tokens,text,scrollbar}.rs`.
use super::{
    Config, Hits, Mode, Sort, WorkspaceHit,
    palette::Palette,
    snapshot::{Snapshot, Status, Tab, Workspace},
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};
use std::collections::{HashMap, HashSet};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const WORKSPACE_HEADER_ROWS: u16 = 2;
pub const MIN_TAB_WIDTH: u16 = 8;
pub const NEW_TAB_WIDTH: u16 = 3;
const TAB_SCROLL_BUTTON_WIDTH: u16 = 3;
const MIN_TAB_STRIP_WIDTH: u16 =
    MIN_TAB_WIDTH + NEW_TAB_WIDTH + TAB_SCROLL_BUTTON_WIDTH.saturating_mul(2);

/// Scroll and selection state the chrome reads and adjusts while drawing.
#[derive(Default)]
pub struct ChromeState {
    pub workspace_scroll: usize,
    pub agent_scroll: usize,
    pub tab_scroll: usize,
    pub reveal_focused_workspace: bool,
    pub reveal_focused_tab: bool,
    pub collapsed_groups: HashSet<String>,
    pub section_split: f32,
    pub selected_workspace_id: Option<String>,
    /// The space being dragged, drawn highlighted while it moves.
    pub dragged_workspace_id: Option<String>,
    /// The row between spaces where a dragged space would land.
    pub workspace_drop_row: Option<u16>,
    /// The gap between tabs where a dragged tab would land.
    pub tab_drop_index: Option<usize>,
}

pub fn display_width(text: &str) -> u16 {
    UnicodeWidthStr::width(text).min(u16::MAX as usize) as u16
}

pub fn truncate_end(text: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let mut output = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > max_width - 1 {
            break;
        }
        output.push(ch);
        width += ch_width;
    }
    format!("{output}…")
}

pub fn put_text(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    if width == 0 || y >= buffer.area.bottom() || x >= buffer.area.right() {
        return;
    }
    buffer.set_stringn(x, y, text, width as usize, style);
}

pub fn put_right_text(buffer: &mut Buffer, area: Rect, y: u16, text: &str, style: Style) {
    let width = display_width(text).min(area.width);
    put_text(
        buffer,
        area.right().saturating_sub(width),
        y,
        width,
        text,
        style,
    );
}

fn put_segment(buffer: &mut Buffer, x: u16, y: u16, right: u16, text: &str, style: Style) -> u16 {
    let width = display_width(text).min(right.saturating_sub(x));
    put_text(buffer, x, y, width, text, style);
    x.saturating_add(width)
}

pub fn status_icon(status: Status, symbols: bool) -> &'static str {
    match (symbols, status) {
        (false, Status::Working | Status::Blocked | Status::Done) => "●",
        (_, Status::Idle) => "○",
        (_, Status::Unknown) => "·",
        (true, Status::Blocked) => "×",
        (true, Status::Working) => "◐",
        (true, Status::Done) => "✓",
    }
}

pub fn status_color(status: Status, palette: &Palette) -> Color {
    match status {
        Status::Working => palette.yellow,
        Status::Blocked => palette.red,
        Status::Done => palette.teal,
        Status::Idle => palette.green,
        Status::Unknown => palette.overlay0,
    }
}

// ---- list scrolling -------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollMetrics {
    pub offset_from_bottom: usize,
    pub max_offset_from_bottom: usize,
    pub viewport_rows: usize,
}

pub fn list_scroll_metrics(
    row_heights: &[u16],
    gaps_after: &[u16],
    body_height: u16,
    requested_start: usize,
) -> ScrollMetrics {
    if row_heights.is_empty() || body_height == 0 {
        return ScrollMetrics::default();
    }
    let mut used = 0u16;
    let mut max_start = row_heights.len();
    for index in (0..row_heights.len()).rev() {
        let height = row_heights[index].max(1).min(body_height);
        let gap = gaps_after.get(index).copied().unwrap_or(0);
        if used.saturating_add(height).saturating_add(gap) > body_height {
            break;
        }
        used = used.saturating_add(height).saturating_add(gap);
        max_start = index;
    }
    max_start = max_start.min(row_heights.len().saturating_sub(1));
    let start = requested_start.min(max_start);
    let mut viewport_rows = 0usize;
    let mut used = 0u16;
    for (index, row_height) in row_heights.iter().enumerate().skip(start) {
        let height = (*row_height).max(1).min(body_height);
        if used.saturating_add(height) > body_height {
            break;
        }
        used = used.saturating_add(height);
        viewport_rows += 1;
        let gap = gaps_after.get(index).copied().unwrap_or(0);
        if used.saturating_add(gap) > body_height {
            break;
        }
        used = used.saturating_add(gap);
    }
    ScrollMetrics {
        offset_from_bottom: max_start.saturating_sub(start),
        max_offset_from_bottom: max_start,
        viewport_rows,
    }
}

fn list_scroll_start_to_reveal(
    row_heights: &[u16],
    gaps_after: &[u16],
    body_height: u16,
    requested_start: usize,
    target: usize,
) -> usize {
    let mut metrics = list_scroll_metrics(row_heights, gaps_after, body_height, requested_start);
    let mut start = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    if target < start {
        return target;
    }
    while target >= start.saturating_add(metrics.viewport_rows)
        && start < metrics.max_offset_from_bottom
    {
        start = start.saturating_add(1);
        metrics = list_scroll_metrics(row_heights, gaps_after, body_height, start);
    }
    start
}

pub struct Thumb {
    pub top: u16,
    pub len: u16,
}

pub fn scrollbar_thumb(metrics: ScrollMetrics, track: Rect) -> Option<Thumb> {
    if metrics.max_offset_from_bottom == 0 || track.height == 0 {
        return None;
    }
    let track_height = track.height as usize;
    let total_rows = metrics.max_offset_from_bottom + metrics.viewport_rows;
    if total_rows == 0 {
        return None;
    }
    let thumb_len = ((metrics.viewport_rows * track_height) as f32 / total_rows as f32)
        .round()
        .max(1.0)
        .min(track_height as f32) as usize;
    let max_thumb_top = track_height.saturating_sub(thumb_len);
    let scrolled_from_top = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let thumb_top = if max_thumb_top == 0 {
        0
    } else {
        ((scrolled_from_top * max_thumb_top) as f32 / metrics.max_offset_from_bottom as f32)
            .round()
            .clamp(0.0, max_thumb_top as f32) as usize
    };
    Some(Thumb {
        top: track.y + thumb_top as u16,
        len: thumb_len as u16,
    })
}

fn render_list_scrollbar(buffer: &mut Buffer, track: Rect, metrics: ScrollMetrics, p: &Palette) {
    let Some(thumb) = scrollbar_thumb(metrics, track) else {
        return;
    };
    for row in track.y..track.bottom() {
        if let Some(cell) = buffer.cell_mut((track.x, row)) {
            cell.set_symbol("▕")
                .set_style(Style::default().fg(p.surface_dim));
        }
    }
    for row in thumb.top..thumb.top.saturating_add(thumb.len) {
        if let Some(cell) = buffer.cell_mut((track.x, row)) {
            cell.set_symbol("▕")
                .set_style(Style::default().fg(p.overlay0));
        }
    }
}

// ---- sidebar tokens -------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    StateIcon,
    Machine(String),
    Workspace(String),
    Tab(String),
    Agent(String),
    Branch(String),
    GitStatus { ahead: usize, behind: usize },
}

impl Token {
    fn text(&self) -> Option<&str> {
        match self {
            Self::Machine(text)
            | Self::Workspace(text)
            | Self::Tab(text)
            | Self::Agent(text)
            | Self::Branch(text) => Some(text),
            Self::StateIcon | Self::GitStatus { .. } => None,
        }
    }
}

fn separator(previous: &Token, current: &Token) -> &'static str {
    if matches!(previous, Token::StateIcon) || matches!(current, Token::GitStatus { .. }) {
        " "
    } else {
        " · "
    }
}

/// Herdr's default space rows: the state icon and name, then the branch and its divergence.
pub fn workspace_rows(workspace: &Workspace, indented: bool) -> Vec<Vec<Token>> {
    let label = if indented && !workspace.custom_label {
        workspace
            .branch
            .as_deref()
            .map(|branch| branch.strip_prefix("worktree/").unwrap_or(branch))
            .unwrap_or(&workspace.label)
    } else {
        &workspace.label
    };
    let mut rows = vec![vec![Token::StateIcon, Token::Workspace(label.into())]];
    if !indented {
        let mut second = vec![];
        if let Some(branch) = &workspace.branch {
            second.push(Token::Branch(branch.clone()));
        }
        if let Some((ahead, behind)) = workspace
            .git_ahead_behind
            .filter(|(ahead, behind)| *ahead > 0 || *behind > 0)
        {
            second.push(Token::GitStatus { ahead, behind });
        }
        if !second.is_empty() {
            rows.push(second);
        }
    }
    rows
}

#[allow(clippy::too_many_arguments)]
fn token_spans(
    tokens: &[Token],
    icon: (&str, Style),
    primary: Style,
    secondary: Style,
    palette: &Palette,
    max_width: usize,
) -> Vec<Span<'static>> {
    let width = |text: &str| UnicodeWidthStr::width(text);
    let fixed: Vec<usize> = tokens
        .iter()
        .map(|token| match token {
            Token::StateIcon => width(icon.0),
            Token::GitStatus { ahead, behind } => {
                usize::from(*ahead > 0) * width(&format!("↑{ahead}"))
                    + usize::from(*behind > 0) * width(&format!("↓{behind}"))
                    + usize::from(*ahead > 0 && *behind > 0)
            }
            _ => 0,
        })
        .collect();
    let flexible: Vec<usize> = tokens
        .iter()
        .map(|token| token.text().map_or(0, width))
        .collect();
    let minimum_width = |active: &[bool]| {
        let indices: Vec<usize> = (0..tokens.len()).filter(|i| active[*i]).collect();
        let content: usize = indices
            .iter()
            .map(|i| fixed[*i] + usize::from(flexible[*i] > 0))
            .sum();
        let separators: usize = indices
            .windows(2)
            .map(|pair| width(separator(&tokens[pair[0]], &tokens[pair[1]])))
            .sum();
        content + separators
    };
    let mut active = vec![true; tokens.len()];
    if minimum_width(&active) > max_width {
        for (index, w) in flexible.iter().enumerate() {
            if *w > 0 {
                active[index] = false;
            }
        }
        for index in (0..tokens.len()).rev() {
            if flexible[index] == 0 {
                continue;
            }
            active[index] = true;
            if minimum_width(&active) > max_width {
                active[index] = false;
            }
        }
    }
    let visible: Vec<usize> = (0..tokens.len()).filter(|i| active[*i]).collect();
    let separator_width: usize = visible
        .windows(2)
        .map(|pair| width(separator(&tokens[pair[0]], &tokens[pair[1]])))
        .sum();
    let fixed_width: usize = visible.iter().map(|i| fixed[*i]).sum();
    let mut budgets: Vec<usize> = flexible
        .iter()
        .enumerate()
        .map(|(i, w)| usize::from(active[i] && *w > 0))
        .collect();
    let minimum: usize = budgets.iter().sum();
    let mut remaining = max_width
        .saturating_sub(separator_width + fixed_width)
        .saturating_sub(minimum);
    while remaining > 0 {
        let mut grew = false;
        for (budget, w) in budgets.iter_mut().zip(&flexible) {
            if *budget > 0 && *budget < *w {
                *budget += 1;
                remaining -= 1;
                grew = true;
                if remaining == 0 {
                    break;
                }
            }
        }
        if !grew {
            break;
        }
    }
    let mut spans = Vec::new();
    for (position, index) in visible.iter().copied().enumerate() {
        let token = &tokens[index];
        if position > 0 {
            spans.push(Span::styled(
                separator(&tokens[visible[position - 1]], token),
                Style::default().fg(palette.overlay0),
            ));
        }
        match token {
            Token::StateIcon => spans.push(Span::styled(icon.0.to_string(), icon.1)),
            Token::Workspace(text) => {
                spans.push(Span::styled(truncate_end(text, budgets[index]), primary))
            }
            Token::Machine(text) | Token::Tab(text) | Token::Agent(text) | Token::Branch(text) => {
                spans.push(Span::styled(truncate_end(text, budgets[index]), secondary))
            }
            Token::GitStatus { ahead, behind } => {
                if *ahead > 0 {
                    spans.push(Span::styled(
                        format!("↑{ahead}"),
                        Style::default().fg(palette.green),
                    ));
                }
                if *ahead > 0 && *behind > 0 {
                    spans.push(Span::raw(" "));
                }
                if *behind > 0 {
                    spans.push(Span::styled(
                        format!("↓{behind}"),
                        Style::default().fg(palette.red),
                    ));
                }
            }
        }
    }
    spans
}

// ---- sidebar --------------------------------------------------------------

fn sidebar_section_heights(total_height: u16, split_ratio: f32) -> (u16, u16) {
    if total_height == 0 {
        return (0, 0);
    }
    if total_height < 6 {
        let workspace_height = total_height.div_ceil(2);
        return (
            workspace_height,
            total_height.saturating_sub(workspace_height),
        );
    }
    let workspace_height = ((total_height as f32) * split_ratio.clamp(0.1, 0.9)).round() as u16;
    let workspace_height = workspace_height.clamp(3, total_height.saturating_sub(3));
    (
        workspace_height,
        total_height.saturating_sub(workspace_height),
    )
}

pub fn expanded_sidebar_sections(area: Rect, split_ratio: f32) -> (Rect, Rect) {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.is_empty() {
        return (Rect::default(), Rect::default());
    }
    let (workspace_height, detail_height) = sidebar_section_heights(content.height, split_ratio);
    (
        Rect::new(content.x, content.y, content.width, workspace_height),
        Rect::new(
            content.x,
            content.y + workspace_height,
            content.width,
            detail_height,
        ),
    )
}

fn sidebar_section_divider_rect(area: Rect, split_ratio: f32) -> Rect {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height < 6 {
        return Rect::default();
    }
    let (workspace_height, _) = sidebar_section_heights(content.height, split_ratio);
    Rect::new(content.x, content.y + workspace_height, content.width, 1)
}

fn render_sidebar_background(buffer: &mut Buffer, area: Rect, palette: &Palette) {
    buffer.set_style(area, Style::default().bg(palette.sidebar_bg));
    let separator_x = area.right().saturating_sub(1);
    for y in area.y..area.bottom() {
        if let Some(cell) = buffer.cell_mut((separator_x, y)) {
            cell.set_symbol("│");
            cell.set_style(Style::default().fg(palette.surface_dim));
        }
    }
}

fn workspace_selection_background(palette: &Palette) -> Color {
    if palette.selection_bg == Color::Reset {
        palette.active_row_bg
    } else {
        palette.selection_bg
    }
}

fn workspace_active_background(palette: &Palette, navigating: bool) -> Color {
    if navigating && palette.selection_bg == Color::Reset {
        palette.sidebar_bg
    } else {
        palette.active_row_bg
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkspaceEntry {
    pub index: usize,
    pub indented: bool,
    pub last_child: bool,
}

/// Orders spaces with linked worktrees grouped under their main checkout.
pub fn workspace_entries(
    snapshot: &Snapshot,
    collapsed_groups: &HashSet<String>,
) -> Vec<WorkspaceEntry> {
    let mut members = HashMap::<&str, Vec<usize>>::new();
    for (index, workspace) in snapshot.workspaces.iter().enumerate() {
        if let Some(worktree) = &workspace.worktree {
            members.entry(&worktree.key).or_default().push(index);
        }
    }
    let linked = |index: &usize| {
        snapshot.workspaces[*index]
            .worktree
            .as_ref()
            .is_some_and(|w| w.is_linked_worktree)
    };
    let grouped: HashSet<&str> = members
        .iter()
        .filter(|(_, indices)| {
            indices.iter().any(linked) && indices.iter().any(|index| !linked(index))
        })
        .map(|(key, _)| *key)
        .collect();
    let mut emitted = HashSet::<&str>::new();
    let mut entries = Vec::new();
    for (index, workspace) in snapshot.workspaces.iter().enumerate() {
        let Some(worktree) = workspace
            .worktree
            .as_ref()
            .filter(|w| grouped.contains(w.key.as_str()))
        else {
            entries.push(WorkspaceEntry {
                index,
                indented: false,
                last_child: false,
            });
            continue;
        };
        if !emitted.insert(&worktree.key) {
            continue;
        }
        let Some(group) = members.get(worktree.key.as_str()) else {
            continue;
        };
        for parent in group.iter().copied().filter(|m| !linked(m)) {
            entries.push(WorkspaceEntry {
                index: parent,
                indented: false,
                last_child: false,
            });
        }
        if collapsed_groups.contains(&worktree.key) {
            if let Some(active) = group
                .iter()
                .copied()
                .find(|m| snapshot.workspaces[*m].focused && linked(m))
            {
                entries.push(WorkspaceEntry {
                    index: active,
                    indented: true,
                    last_child: true,
                });
            }
            continue;
        }
        let children: Vec<usize> = group.iter().copied().filter(linked).collect();
        for (child_index, child) in children.iter().enumerate() {
            entries.push(WorkspaceEntry {
                index: *child,
                indented: true,
                last_child: child_index + 1 == children.len(),
            });
        }
    }
    entries
}

/// The group a main checkout heads, when it has linked worktrees open.
pub fn parent_group_key(snapshot: &Snapshot, index: usize) -> Option<String> {
    let worktree = snapshot.workspaces.get(index)?.worktree.as_ref()?;
    if worktree.is_linked_worktree {
        return None;
    }
    snapshot
        .workspaces
        .iter()
        .any(|candidate| {
            candidate
                .worktree
                .as_ref()
                .is_some_and(|c| c.key == worktree.key && c.is_linked_worktree)
        })
        .then(|| worktree.key.clone())
}

/// Whether closing this main checkout also closes its linked worktree spaces.
pub fn workspace_close_is_group(snapshot: &Snapshot, workspace: &Workspace) -> bool {
    let Some(worktree) = workspace
        .worktree
        .as_ref()
        .filter(|w| !w.is_linked_worktree)
    else {
        return false;
    };
    let mut has_child = false;
    for member in &snapshot.workspaces {
        if member.workspace_id == workspace.workspace_id {
            continue;
        }
        if let Some(candidate) = member.worktree.as_ref().filter(|c| c.key == worktree.key) {
            if !candidate.is_linked_worktree {
                return false;
            }
            has_child = true;
        }
    }
    has_child
}

fn displayed_workspace_status(
    snapshot: &Snapshot,
    workspace: &Workspace,
    collapsed_groups: &HashSet<String>,
) -> Status {
    let Some(worktree) = workspace
        .worktree
        .as_ref()
        .filter(|w| !w.is_linked_worktree)
    else {
        return workspace.agent_status;
    };
    if !collapsed_groups.contains(&worktree.key) {
        return workspace.agent_status;
    }
    snapshot
        .workspaces
        .iter()
        .filter(|candidate| {
            candidate.worktree.as_ref().is_some_and(|member| {
                member.key == worktree.key
                    && (member.is_linked_worktree
                        || candidate.workspace_id == workspace.workspace_id)
            })
        })
        .map(|candidate| candidate.agent_status)
        .max_by_key(|status| status.priority())
        .unwrap_or(workspace.agent_status)
}

#[allow(clippy::too_many_arguments)]
fn render_workspace_rows(
    buffer: &mut Buffer,
    area: Rect,
    status: Status,
    config: &Config,
    entry: &WorkspaceEntry,
    rows: Vec<Vec<Token>>,
    focused: bool,
    selected: bool,
    navigating: bool,
    dragged: bool,
) {
    let palette = &config.palette;
    let highlighted = focused || dragged;
    for (row_index, row) in rows.iter().enumerate() {
        let y = area.y + row_index as u16;
        if y >= area.bottom() {
            break;
        }
        let mut x = area.x;
        if entry.indented {
            let prefix = match (row_index == 0, entry.last_child) {
                (true, true) => "   └─ ",
                (true, false) => "   ├─ ",
                (false, true) => "        ",
                (false, false) => "   │    ",
            };
            x = put_segment(
                buffer,
                x,
                y,
                area.right(),
                prefix,
                Style::default().fg(palette.overlay0),
            );
        } else if row_index == 0 {
            x = x.saturating_add(1);
        } else {
            x = x.saturating_add(3);
        }
        let workspace_style = if highlighted {
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.subtext0)
        };
        let secondary_style = Style::default().fg(if focused {
            palette.mauve
        } else {
            palette.overlay0
        });
        let spans = token_spans(
            row,
            (
                status_icon(status, config.symbols),
                Style::default().fg(status_color(status, palette)),
            ),
            workspace_style,
            secondary_style,
            palette,
            area.right().saturating_sub(2).saturating_sub(x) as usize,
        );
        Paragraph::new(Line::from(spans)).render(
            Rect::new(x, y, area.right().saturating_sub(2).saturating_sub(x), 1),
            buffer,
        );
    }
    let background = if selected {
        Some(workspace_selection_background(palette))
    } else if dragged {
        Some(palette.surface1)
    } else if focused {
        Some(workspace_active_background(palette, navigating))
    } else {
        None
    };
    if let Some(background) = background {
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                buffer[(x, y)].set_bg(background);
            }
        }
    }
}

fn render_parent_group_toggle(
    buffer: &mut Buffer,
    rect: Rect,
    snapshot: &Snapshot,
    index: usize,
    collapsed_groups: &HashSet<String>,
    palette: &Palette,
) -> Option<(Rect, String)> {
    let key = parent_group_key(snapshot, index)?;
    let toggle = Rect::new(rect.right().saturating_sub(1), rect.y, 1, 1);
    put_text(
        buffer,
        toggle.x,
        toggle.y,
        toggle.width,
        if collapsed_groups.contains(&key) {
            "▸"
        } else {
            "▾"
        },
        Style::default().fg(palette.accent),
    );
    Some((toggle, key))
}

pub fn render_sidebar(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &Snapshot,
    config: &Config,
    state: &mut ChromeState,
    hits: &mut Hits,
) {
    let palette = &config.palette;
    render_sidebar_background(buffer, area, palette);
    hits.sidebar_divider = if area.is_empty() {
        Rect::default()
    } else {
        Rect::new(area.right().saturating_sub(1), area.y, 1, area.height)
    };
    let (workspace_area, detail_area) = expanded_sidebar_sections(area, state.section_split);
    hits.sidebar_section_divider = sidebar_section_divider_rect(area, state.section_split);
    put_text(
        buffer,
        workspace_area.x,
        workspace_area.y,
        workspace_area.width,
        " spaces",
        Style::default()
            .fg(palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );
    let entries = workspace_entries(snapshot, &state.collapsed_groups);
    let body = Rect::new(
        workspace_area.x,
        workspace_area.y.saturating_add(WORKSPACE_HEADER_ROWS),
        workspace_area.width,
        workspace_area
            .height
            .saturating_sub(WORKSPACE_HEADER_ROWS + 1),
    );
    hits.workspace_body = body;
    let rows_for =
        |entry: &WorkspaceEntry| workspace_rows(&snapshot.workspaces[entry.index], entry.indented);
    let row_heights: Vec<u16> = entries
        .iter()
        .map(|entry| rows_for(entry).len().clamp(1, u16::MAX as usize) as u16)
        .collect();
    let gaps = vec![0; entries.len()];
    let mut metrics = list_scroll_metrics(&row_heights, &gaps, body.height, state.workspace_scroll);
    if !body.is_empty() && std::mem::take(&mut state.reveal_focused_workspace) {
        let target = entries.iter().position(|entry| {
            let workspace = &snapshot.workspaces[entry.index];
            match &state.selected_workspace_id {
                Some(selected) => &workspace.workspace_id == selected,
                None => workspace.focused,
            }
        });
        if let Some(target) = target {
            state.workspace_scroll = list_scroll_start_to_reveal(
                &row_heights,
                &gaps,
                body.height,
                state.workspace_scroll,
                target,
            );
            metrics = list_scroll_metrics(&row_heights, &gaps, body.height, state.workspace_scroll);
        }
    }
    hits.workspace_scroll_metrics = Some(metrics);
    state.workspace_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));
    let mut y = body.y;
    for entry in entries.iter().skip(state.workspace_scroll) {
        let workspace = &snapshot.workspaces[entry.index];
        let status = displayed_workspace_status(snapshot, workspace, &state.collapsed_groups);
        let rows = rows_for(entry);
        let row_height = (rows.len().max(1) as u16).min(body.height);
        if y.saturating_add(row_height) > body.bottom() {
            break;
        }
        let rect = Rect::new(body.x, y, content_width, row_height);
        let selected = state.selected_workspace_id.as_deref() == Some(&workspace.workspace_id);
        let dragged = state.dragged_workspace_id.as_deref() == Some(&workspace.workspace_id);
        if selected {
            buffer.set_style(rect, Style::default().bg(palette.selection_bg));
        } else if dragged {
            buffer.set_style(rect, Style::default().bg(palette.surface1));
        } else if workspace.focused {
            buffer.set_style(rect, Style::default().bg(palette.active_row_bg));
        }
        render_workspace_rows(
            buffer,
            rect,
            status,
            config,
            entry,
            rows,
            workspace.focused,
            selected,
            state.selected_workspace_id.is_some(),
            dragged,
        );
        let group_toggle = render_parent_group_toggle(
            buffer,
            rect,
            snapshot,
            entry.index,
            &state.collapsed_groups,
            palette,
        );
        hits.workspaces.push(WorkspaceHit {
            rect,
            workspace_id: workspace.workspace_id.clone(),
            indented: entry.indented,
            group_toggle,
        });
        y = y.saturating_add(row_height);
    }
    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.workspace_scrollbar = track;
        render_list_scrollbar(buffer, track, metrics, palette);
    }
    if let Some(row) = state.workspace_drop_row.filter(|row| {
        *row >= workspace_area.y.saturating_add(1)
            && *row < workspace_area.bottom().saturating_sub(1)
    }) {
        put_text(
            buffer,
            body.x,
            row,
            body.width,
            &"─".repeat(body.width as usize),
            Style::default().fg(palette.accent),
        );
    }
    let footer_y = workspace_area.bottom().saturating_sub(1);
    hits.new_workspace = Rect::new(
        workspace_area.x,
        footer_y,
        5.min(workspace_area.width),
        u16::from(workspace_area.height > 0),
    );
    put_text(
        buffer,
        workspace_area.x,
        footer_y,
        workspace_area.width,
        " new",
        Style::default().fg(palette.overlay0),
    );
    hits.global_launcher = Rect::new(
        workspace_area.right().saturating_sub(6),
        footer_y,
        6.min(workspace_area.width),
        1,
    );
    put_right_text(
        buffer,
        workspace_area,
        footer_y,
        "menu",
        Style::default().fg(palette.overlay0),
    );
    render_agent_panel(buffer, detail_area, snapshot, config, state, hits);
    hits.sidebar_toggle = Rect::new(
        area.right().saturating_sub(2),
        area.bottom().saturating_sub(1),
        u16::from(area.width > 1),
        u16::from(area.height > 0),
    );
    put_text(
        buffer,
        hits.sidebar_toggle.x,
        hits.sidebar_toggle.y,
        hits.sidebar_toggle.width,
        "«",
        Style::default().fg(palette.overlay0),
    );
}

pub fn collapsed_sidebar_sections(area: Rect) -> (Rect, Option<u16>, Rect) {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.is_empty() {
        return (Rect::default(), None, Rect::default());
    }
    if content.height < 7 {
        return (content, None, Rect::default());
    }
    let workspace_height = content.height.div_ceil(2);
    let divider_y = content.y + workspace_height;
    let detail_height = content.height.saturating_sub(workspace_height + 1);
    (
        Rect::new(content.x, content.y, content.width, workspace_height),
        Some(divider_y),
        Rect::new(content.x, divider_y + 1, content.width, detail_height),
    )
}

pub fn render_collapsed_sidebar(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &Snapshot,
    config: &Config,
    selected_workspace_id: Option<&str>,
    hits: &mut Hits,
) {
    let palette = &config.palette;
    let selection_background = workspace_selection_background(palette);
    let active_background = workspace_active_background(palette, selected_workspace_id.is_some());
    render_sidebar_background(buffer, area, palette);
    let (workspace_area, divider_y, detail_area) = collapsed_sidebar_sections(area);
    for (index, workspace) in snapshot
        .workspaces
        .iter()
        .take(workspace_area.height as usize)
        .enumerate()
    {
        let rect = Rect::new(
            workspace_area.x,
            workspace_area.y + index as u16,
            workspace_area.width,
            1,
        );
        let selected = selected_workspace_id == Some(workspace.workspace_id.as_str());
        if selected {
            buffer.set_style(rect, Style::default().bg(selection_background));
        } else if workspace.focused {
            buffer.set_style(rect, Style::default().bg(active_background));
        }
        let number_style = if selected {
            Style::default()
                .fg(palette.overlay1)
                .bg(selection_background)
        } else if workspace.focused {
            Style::default().fg(palette.text).bg(active_background)
        } else {
            Style::default().fg(palette.overlay0)
        };
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width.min(2),
            &format!("{:<2}", index + 1),
            number_style,
        );
        let status = workspace.agent_status;
        put_text(
            buffer,
            rect.x.saturating_add(2),
            rect.y,
            rect.width.saturating_sub(2),
            status_icon(status, config.symbols),
            Style::default().fg(status_color(status, palette)),
        );
        hits.workspaces.push(WorkspaceHit {
            rect,
            workspace_id: workspace.workspace_id.clone(),
            indented: false,
            group_toggle: None,
        });
    }
    if let Some(divider_y) = divider_y {
        put_text(
            buffer,
            workspace_area.x,
            divider_y,
            workspace_area.width,
            &"─".repeat(workspace_area.width as usize),
            Style::default().fg(palette.surface_dim),
        );
    }
    let detail_content = Rect::new(
        detail_area.x,
        detail_area.y,
        detail_area.width,
        detail_area.height.saturating_sub(1),
    );
    for (index, agent) in ordered_agents(snapshot, config.sort)
        .into_iter()
        .take(detail_content.height as usize)
        .enumerate()
    {
        let rect = Rect::new(
            detail_content.x,
            detail_content.y + index as u16,
            detail_content.width,
            1,
        );
        if agent.focused {
            buffer.set_style(rect, Style::default().bg(palette.active_row_bg));
        }
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width.min(2),
            &format!("{:<2}", index + 1),
            Style::default().fg(if agent.focused {
                palette.text
            } else {
                palette.overlay0
            }),
        );
        put_text(
            buffer,
            rect.x.saturating_add(2),
            rect.y,
            rect.width.saturating_sub(2),
            status_icon(agent.agent_status, config.symbols),
            Style::default().fg(status_color(agent.agent_status, palette)),
        );
        hits.agents.push((rect, agent.pane_id.clone()));
    }
    hits.sidebar_toggle = if area.is_empty() || workspace_area.width == 0 {
        Rect::default()
    } else {
        Rect::new(
            workspace_area.x + workspace_area.width / 2,
            area.bottom().saturating_sub(1),
            1,
            1,
        )
    };
    put_text(
        buffer,
        hits.sidebar_toggle.x,
        hits.sidebar_toggle.y,
        hits.sidebar_toggle.width,
        "»",
        Style::default().fg(palette.overlay0),
    );
}

// ---- agents ---------------------------------------------------------------

pub fn ordered_agents(snapshot: &Snapshot, sort: Sort) -> Vec<&super::snapshot::Agent> {
    let mut agents: Vec<_> = snapshot.agents.iter().collect();
    if sort == Sort::Priority {
        agents.sort_by_key(|agent| {
            (
                std::cmp::Reverse(agent.agent_status.priority()),
                std::cmp::Reverse(agent.state_change_seq),
            )
        });
    }
    agents
}

fn agent_rows(snapshot: &Snapshot, agent: &super::snapshot::Agent) -> Vec<Vec<Token>> {
    let tab = agent.tab_id.as_deref().and_then(|id| snapshot.tab(id));
    let tab_count = agent.workspace_id.as_deref().map_or(0, |space| {
        snapshot
            .tabs
            .iter()
            .filter(|t| t.workspace_id == space)
            .count()
    });
    let mut first = vec![Token::StateIcon];
    if agent.workspace_id.is_none() {
        first.push(Token::Machine("no pane".into()));
    }
    first.push(Token::Workspace(agent.workspace_label.clone()));
    if let Some(tab) = tab.filter(|tab| tab_count > 1 || tab.custom_label) {
        first.push(Token::Tab(tab.label.clone()));
    }
    let mut rows = vec![first];
    if let Some(label) = agent.agent.clone().or_else(|| agent.title.clone()) {
        rows.push(vec![Token::Agent(label)]);
    }
    rows
}

fn render_agent_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &Snapshot,
    config: &Config,
    state: &mut ChromeState,
    hits: &mut Hits,
) {
    let palette = &config.palette;
    if area.height == 0 {
        return;
    }
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        &"─".repeat(area.width as usize),
        Style::default().fg(palette.surface_dim),
    );
    if area.height < 2 {
        return;
    }
    put_text(
        buffer,
        area.x,
        area.y + 1,
        area.width,
        " agents",
        Style::default()
            .fg(palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );
    let sort_label = match config.sort {
        Sort::Spaces => "grouped",
        Sort::Priority => "priority",
    };
    let sort_width = display_width(sort_label).min(area.width);
    hits.agent_sort_toggle = Rect::new(
        area.right().saturating_sub(sort_width),
        area.y + 1,
        sort_width,
        1,
    );
    put_text(
        buffer,
        hits.agent_sort_toggle.x,
        hits.agent_sort_toggle.y,
        hits.agent_sort_toggle.width,
        sort_label,
        Style::default()
            .fg(palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );
    let body = Rect::new(
        area.x,
        area.y.saturating_add(3),
        area.width,
        area.height.saturating_sub(3),
    );
    hits.agent_body = body;
    let agents = ordered_agents(snapshot, config.sort);
    if body.is_empty() || agents.is_empty() {
        state.agent_scroll = 0;
        return;
    }
    let rows: Vec<_> = agents
        .iter()
        .map(|agent| (*agent, agent_rows(snapshot, agent)))
        .collect();
    let row_heights: Vec<u16> = rows
        .iter()
        .map(|(_, rows)| rows.len().max(1) as u16)
        .collect();
    let gaps = vec![0; rows.len()];
    let metrics = list_scroll_metrics(&row_heights, &gaps, body.height, state.agent_scroll);
    hits.agent_scroll_metrics = Some(metrics);
    state.agent_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));
    let mut y = body.y;
    for (index, (agent, tokens)) in rows.iter().enumerate().skip(state.agent_scroll) {
        let height = row_heights[index].min(body.height);
        if y.saturating_add(height) > body.bottom() {
            break;
        }
        let rect = Rect::new(body.x, y, content_width, height);
        hits.agents.push((rect, agent.pane_id.clone()));
        render_agent_row(buffer, rect, agent, tokens, config);
        y = y.saturating_add(height);
    }
    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.agent_scrollbar = track;
        render_list_scrollbar(buffer, track, metrics, palette);
    }
}

fn render_agent_row(
    buffer: &mut Buffer,
    rect: Rect,
    agent: &super::snapshot::Agent,
    rows: &[Vec<Token>],
    config: &Config,
) {
    let palette = &config.palette;
    let row_style = if agent.focused {
        Style::default().bg(palette.active_row_bg)
    } else {
        Style::default()
    };
    let name_style = Style::default()
        .fg(if agent.focused {
            palette.text
        } else {
            palette.subtext0
        })
        .add_modifier(Modifier::BOLD);
    let secondary = Style::default().fg(palette.overlay0);
    let icon = (
        status_icon(agent.agent_status, config.symbols),
        Style::default().fg(status_color(agent.agent_status, palette)),
    );
    for (index, tokens) in rows.iter().take(rect.height as usize).enumerate() {
        let indent = if index == 0 { 1 } else { 3 };
        let mut spans = vec![Span::raw(" ".repeat(indent))];
        spans.extend(token_spans(
            tokens,
            icon,
            name_style,
            secondary,
            palette,
            rect.width.saturating_sub(indent as u16) as usize,
        ));
        Paragraph::new(Line::from(spans)).style(row_style).render(
            Rect::new(rect.x, rect.y + index as u16, rect.width, 1),
            buffer,
        );
    }
}

// ---- tab bar --------------------------------------------------------------

fn tab_label(tab: &Tab) -> String {
    if tab.zoomed {
        format!("{} Z", tab.label)
    } else {
        tab.label.clone()
    }
}

pub fn render_tab_bar(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &Snapshot,
    config: &Config,
    state: &mut ChromeState,
    hits: &mut Hits,
) {
    let palette = &config.palette;
    buffer.set_style(area, Style::default().bg(palette.panel_bg));
    let tabs: Vec<&Tab> = snapshot
        .tabs
        .iter()
        .filter(|tab| Some(tab.workspace_id.as_str()) == snapshot.focused_workspace_id.as_deref())
        .collect();
    let desired_widths: Vec<u16> = tabs
        .iter()
        .map(|tab| {
            display_width(&tab_label(tab))
                .saturating_add(4)
                .max(MIN_TAB_WIDTH)
        })
        .collect();
    let content = area;
    let desired_total = desired_widths
        .iter()
        .copied()
        .fold(0_u16, u16::saturating_add)
        .saturating_add(tabs.len().saturating_sub(1).min(u16::MAX as usize) as u16)
        .saturating_add(NEW_TAB_WIDTH);
    let overflow = desired_total > content.width && content.width >= MIN_TAB_STRIP_WIDTH;
    let available = if overflow {
        content
            .width
            .saturating_sub(NEW_TAB_WIDTH)
            .saturating_sub(TAB_SCROLL_BUTTON_WIDTH.saturating_mul(2))
    } else {
        content.width.saturating_sub(NEW_TAB_WIDTH)
    };
    let max_scroll = max_tab_scroll(&desired_widths, available);
    if !overflow {
        state.tab_scroll = 0;
    } else if state.reveal_focused_tab {
        if let Some(focused) = tabs.iter().position(|tab| tab.focused) {
            state.tab_scroll =
                centered_tab_scroll(focused, &desired_widths, available).min(max_scroll);
        }
    } else {
        state.tab_scroll = state.tab_scroll.min(max_scroll);
    }
    state.reveal_focused_tab = false;
    let mut x = content.x;
    let tab_right = if overflow {
        hits.tab_scroll_left = Rect::new(
            content.x,
            content.y,
            TAB_SCROLL_BUTTON_WIDTH.min(content.width),
            1,
        );
        put_text(
            buffer,
            hits.tab_scroll_left.x,
            content.y,
            hits.tab_scroll_left.width,
            " < ",
            Style::default()
                .fg(if state.tab_scroll > 0 {
                    palette.overlay1
                } else {
                    palette.overlay0
                })
                .bg(palette.surface0),
        );
        x = hits.tab_scroll_left.right();
        content
            .right()
            .saturating_sub(NEW_TAB_WIDTH + TAB_SCROLL_BUTTON_WIDTH)
    } else {
        content.right().saturating_sub(NEW_TAB_WIDTH)
    };
    let mut first_visible = None;
    let mut last_visible = None;
    for (index, tab) in tabs.iter().enumerate().skip(state.tab_scroll) {
        let name = tab_label(tab);
        let desired = desired_widths[index];
        let width = desired.min(tab_right.saturating_sub(x));
        if width == 0 {
            break;
        }
        let rect = Rect::new(x, area.y, width, 1);
        let style = if tab.focused {
            let base = Style::default().fg(palette.contrast()).bg(palette.accent);
            if tab.custom_label {
                base.add_modifier(Modifier::BOLD)
            } else {
                base
            }
        } else if tab.custom_label {
            Style::default().fg(palette.overlay1).bg(palette.surface0)
        } else {
            Style::default().fg(palette.overlay0).bg(palette.surface0)
        };
        let padding = width.saturating_sub(display_width(&name));
        let left = padding / 2;
        let text = format!(
            "{empty:left$}{name}{empty:right$}",
            empty = "",
            left = left as usize,
            right = padding.saturating_sub(left) as usize,
        );
        put_text(buffer, rect.x, rect.y, rect.width, &text, style);
        hits.tabs.push((rect, tab.tab_id.clone()));
        first_visible.get_or_insert(index);
        last_visible = Some(index);
        x = x.saturating_add(width + 1);
        if width < desired {
            break;
        }
    }
    if overflow {
        hits.tab_scroll_right = Rect::new(tab_right, area.y, TAB_SCROLL_BUTTON_WIDTH, 1);
        put_text(
            buffer,
            hits.tab_scroll_right.x,
            area.y,
            hits.tab_scroll_right.width,
            " > ",
            Style::default()
                .fg(if state.tab_scroll < max_scroll {
                    palette.overlay1
                } else {
                    palette.overlay0
                })
                .bg(palette.surface0),
        );
        hits.new_tab = Rect::new(
            hits.tab_scroll_right.right(),
            area.y,
            content
                .right()
                .saturating_sub(hits.tab_scroll_right.right())
                .min(NEW_TAB_WIDTH),
            1,
        );
    } else {
        hits.new_tab = Rect::new(
            x.min(content.right()),
            area.y,
            content.right().saturating_sub(x).min(NEW_TAB_WIDTH),
            1,
        );
    }
    if snapshot.focused_workspace_id.is_some() {
        put_text(
            buffer,
            hits.new_tab.x,
            area.y,
            hits.new_tab.width,
            " + ",
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
    } else {
        hits.new_tab = Rect::default();
    }
    if first_visible.is_some_and(|index| index > 0) {
        let ellipsis_x = if hits.tab_scroll_left.width > 0 {
            hits.tab_scroll_left.right()
        } else {
            content.x
        };
        put_text(
            buffer,
            ellipsis_x,
            area.y,
            u16::from(ellipsis_x < content.right()),
            "…",
            Style::default().fg(palette.overlay0),
        );
    }
    if last_visible.is_some_and(|index| index + 1 < tabs.len()) {
        let ellipsis_x = if hits.tab_scroll_right.width > 0 {
            hits.tab_scroll_right.x.saturating_sub(1)
        } else {
            content.right().saturating_sub(1)
        };
        put_text(
            buffer,
            ellipsis_x,
            area.y,
            u16::from(ellipsis_x >= content.x && ellipsis_x < content.right()),
            "…",
            Style::default().fg(palette.overlay0),
        );
    }
    if let Some(x) = state
        .tab_drop_index
        .and_then(|insert| tab_drop_indicator_x(hits, &tabs, insert))
    {
        put_text(
            buffer,
            x.min(content.right().saturating_sub(1)),
            area.y,
            1,
            "│",
            Style::default().fg(palette.accent),
        );
    }
}

fn tab_drop_indicator_x(hits: &Hits, tabs: &[&Tab], insert_index: usize) -> Option<u16> {
    let visible: Vec<(usize, Rect)> = hits
        .tabs
        .iter()
        .filter_map(|(rect, id)| {
            tabs.iter()
                .position(|tab| tab.tab_id == *id)
                .map(|index| (index, *rect))
        })
        .collect();
    let (first_index, first_rect) = *visible.first()?;
    let (last_index, last_rect) = *visible.last()?;
    if insert_index == 0 {
        return Some(if first_index == 0 {
            first_rect.x
        } else {
            hits.tab_scroll_left.right()
        });
    }
    if let Some((_, rect)) = visible.iter().find(|(index, _)| *index == insert_index) {
        return Some(rect.x.saturating_sub(1));
    }
    if insert_index >= tabs.len() {
        return Some(if last_index + 1 >= tabs.len() {
            last_rect.right()
        } else {
            hits.tab_scroll_right.x.saturating_sub(1)
        });
    }
    None
}

fn centered_tab_scroll(focused: usize, widths: &[u16], available: u16) -> usize {
    let mut best = focused;
    let mut best_distance = u16::MAX;
    for start in 0..=focused {
        let before = widths
            .iter()
            .copied()
            .skip(start)
            .take(focused.saturating_sub(start))
            .fold(0u16, |width, tab| width.saturating_add(tab + 1));
        if before >= available {
            continue;
        }
        let focused_width = widths[focused].min(available.saturating_sub(before));
        let center = before.saturating_mul(2).saturating_add(focused_width);
        let distance = center.abs_diff(available);
        if distance <= best_distance {
            best_distance = distance;
            best = start;
        }
    }
    best
}

fn max_tab_scroll(widths: &[u16], available: u16) -> usize {
    let Some((&last, preceding)) = widths.split_last() else {
        return 0;
    };
    let mut start = preceding.len();
    let mut used = u32::from(last);
    for width in preceding.iter().rev() {
        let required = used + 1 + u32::from(*width);
        if required > u32::from(available) {
            break;
        }
        used = required;
        start -= 1;
    }
    start
}

// ---- mode bar -------------------------------------------------------------

/// Draws the bottom-row mode bar over the pane area; returns its row.
pub fn render_mode_bar(
    buffer: &mut Buffer,
    pane_area: Rect,
    mode: Mode,
    copy: Option<&super::CopyBar>,
    error: Option<&str>,
    palette: &Palette,
) -> Option<Rect> {
    if (mode == Mode::Terminal && error.is_none()) || pane_area.is_empty() {
        return None;
    }
    let bar = Rect::new(
        pane_area.x,
        pane_area.y + pane_area.height.saturating_sub(1),
        pane_area.width,
        1,
    );
    let base = Style::default().fg(palette.overlay0).bg(palette.panel_bg);
    for x in bar.x..bar.right() {
        buffer[(x, bar.y)].set_symbol(" ").set_style(base);
    }
    let key = Style::default()
        .fg(palette.accent)
        .bg(palette.panel_bg)
        .add_modifier(Modifier::BOLD);
    let mode_style = Style::default()
        .fg(palette.contrast())
        .bg(if mode == Mode::Resize {
            palette.mauve
        } else {
            palette.accent
        })
        .add_modifier(Modifier::BOLD);
    let mut segments = Vec::<(String, Style)>::new();
    if let Some(error) = error.filter(|_| mode == Mode::Terminal) {
        segments.extend([
            (" ERROR ".to_owned(), mode_style),
            (format!(" {error}"), base),
        ]);
    } else {
        let text = |s: &str| s.to_owned();
        match mode {
            Mode::Prefix => segments.extend([
                (text(" PREFIX "), mode_style),
                (text(" "), base),
                (text("esc"), key),
                (text(" cancel  "), base),
                (text("ctrl+b"), key),
                (text(" send prefix  "), base),
                (text("w"), key),
                (text(" workspace nav  "), base),
                (text("?"), key),
                (text(" keybinds"), base),
            ]),
            Mode::Navigate => segments.extend([
                (text(" NAVIGATE "), mode_style),
                (text(" esc back  "), base),
                (text("↑/↓"), key),
                (text(" workspace  "), base),
                (text("tab"), key),
                (text(" pane  "), base),
                (text("?"), key),
                (text(" keybinds"), base),
            ]),
            Mode::Resize => segments.extend([
                (text(" RESIZE "), mode_style),
                (text("  "), base),
                (text("h/l"), key),
                (text(" width  "), base),
                (text("j/k"), key),
                (text(" height  "), base),
                (text("esc"), key),
                (text(" done"), base),
            ]),
            Mode::Copy => {
                let copy = copy?;
                if let Some(query) = &copy.search_prompt {
                    buffer.set_stringn(bar.x, bar.y, " COPY ", usize::from(bar.width), mode_style);
                    if bar.width >= 8 {
                        buffer.set_string(bar.x + 7, bar.y, "/", key);
                    }
                    let footer = "  enter search  esc cancel";
                    let footer_width = if bar.width >= 50 {
                        footer.len() as u16
                    } else {
                        0
                    };
                    let prefix = 8.min(bar.width);
                    put_text(
                        buffer,
                        bar.x + prefix,
                        bar.y,
                        bar.width.saturating_sub(prefix + footer_width),
                        query,
                        Style::default().fg(palette.text).bg(palette.panel_bg),
                    );
                    if footer_width > 0 {
                        buffer.set_string(bar.right() - footer_width, bar.y, footer, base);
                    }
                    return Some(bar);
                }
                let select = if copy.selecting {
                    "selecting"
                } else {
                    "select"
                };
                let match_status = if copy.query.is_empty() {
                    String::new()
                } else if copy.matched {
                    " 1/1".into()
                } else {
                    " 0/0".into()
                };
                let (exit_keys, exit_label) = if copy.query.is_empty() && !copy.selecting {
                    ("q/esc", " exit")
                } else {
                    ("esc", " clear  q exit")
                };
                segments.extend([
                    (text(" COPY "), mode_style),
                    (text(" "), base),
                    (text("h/j/k/l"), key),
                    (text(" move  "), base),
                    (text("/"), key),
                    (text(" search  "), base),
                    (text("n"), key),
                    (format!(" repeat{match_status}  "), base),
                    (text("v/space"), key),
                    (format!(" {select}  "), base),
                    (text("y/enter"), key),
                    (text(" copy  "), base),
                    (text(exit_keys), key),
                    (text(exit_label), base),
                ]);
            }
            Mode::Terminal => {}
        }
    }
    let mut x = bar.x;
    for (text, style) in segments {
        if x >= bar.right() {
            break;
        }
        let remaining = bar.right() - x;
        buffer.set_stringn(x, bar.y, &text, usize::from(remaining), style);
        x = x.saturating_add(display_width(&text).min(remaining));
    }
    Some(bar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::snapshot::Worktree;

    fn workspace(id: &str, key: Option<(&str, bool)>, focused: bool) -> Workspace {
        Workspace {
            workspace_id: id.into(),
            number: 1,
            label: id.into(),
            custom_label: false,
            cwd: String::new(),
            repository: None,
            branch: Some(format!("worktree/{id}")),
            git_ahead_behind: None,
            worktree: key.map(|(key, linked)| Worktree {
                key: key.into(),
                is_linked_worktree: linked,
            }),
            focused,
            agent_status: Status::Unknown,
        }
    }

    #[test]
    fn linked_worktrees_group_under_their_main_checkout() {
        let snapshot = Snapshot {
            workspaces: vec![
                workspace("child", Some(("repo", true)), true),
                workspace("other", None, false),
                workspace("main", Some(("repo", false)), false),
                workspace("child2", Some(("repo", true)), false),
            ],
            ..Snapshot::default()
        };
        let entries: Vec<_> = workspace_entries(&snapshot, &HashSet::new())
            .iter()
            .map(|e| (e.index, e.indented, e.last_child))
            .collect();
        assert_eq!(
            entries,
            vec![
                (2, false, false),
                (0, true, false),
                (3, true, true),
                (1, false, false)
            ]
        );
        let collapsed = HashSet::from(["repo".to_owned()]);
        let entries: Vec<_> = workspace_entries(&snapshot, &collapsed)
            .iter()
            .map(|e| e.index)
            .collect();
        assert_eq!(entries, vec![2, 0, 1]);
        assert!(workspace_close_is_group(&snapshot, &snapshot.workspaces[2]));
        assert_eq!(
            workspace_rows(&snapshot.workspaces[0], true)[0][1],
            Token::Workspace("child".into())
        );
    }

    #[test]
    fn narrow_rows_drop_secondary_text_before_the_name() {
        let tokens = [
            Token::StateIcon,
            Token::Workspace("workspace".into()),
            Token::Tab("tab".into()),
        ];
        let palette = Palette::default();
        let text = |width| {
            token_spans(
                &tokens,
                ("●", Style::default()),
                Style::default(),
                Style::default(),
                &palette,
                width,
            )
            .iter()
            .map(|s| s.content.to_string())
            .collect::<String>()
        };
        assert_eq!(text(40), "● workspace · tab");
        assert_eq!(text(12), "● wor… · tab");
        assert_eq!(text(5), "● tab");
    }

    #[test]
    fn trailing_scroll_limit_accounts_for_full_widths_and_separators() {
        for (widths, available, expected) in [
            (&[][..], 0, 0),
            (&[8, 13][..], 0, 1),
            (&[8, 13][..], 21, 1),
            (&[8, 13][..], 22, 0),
        ] {
            assert_eq!(max_tab_scroll(widths, available), expected);
        }
    }
}
