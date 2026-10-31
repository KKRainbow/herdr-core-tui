//! Herdr's mobile layout for narrow terminals: a two-row header with a `switch`
//! button, and a full-screen switcher for machines, agents, spaces, tabs, and the menu.
//! Adapted from Herdr `src/client/shell/mobile.rs` at ee5bfb5041bb3e4a49d591c72308aae3c81ecdde.
use super::{
    Config, Hits,
    chrome::{
        display_width, ordered_agents, put_segment, put_text, status_color, status_icon,
        truncate_end, workspace_entries,
    },
    overlays::global_menu_items,
    palette::Palette,
    snapshot::{Snapshot, Status, Workspace},
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Widget},
};
use std::collections::HashSet;

const MOBILE_BUTTON_WIDTH: u16 = 10;

/// What a row of the switcher opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MobileTarget {
    Machine(String),
    NewWorkspace,
    Workspace(String),
    Agent(String),
    NewTab,
    Tab(String),
    Menu(usize),
}

struct MobileItem {
    lines: Vec<Line<'static>>,
    background: Color,
    target: Option<MobileTarget>,
}

impl MobileItem {
    fn section(label: impl Into<String>, palette: &Palette) -> Self {
        Self {
            lines: vec![Line::from(Span::styled(
                format!(" {} ", label.into()),
                Style::default()
                    .fg(palette.overlay1)
                    .bg(palette.panel_bg)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ))],
            background: palette.panel_bg,
            target: None,
        }
    }

    fn action(label: &'static str, target: MobileTarget, palette: &Palette) -> Self {
        Self {
            lines: vec![Line::from(Span::styled(
                label,
                Style::default()
                    .fg(palette.accent)
                    .bg(palette.panel_bg)
                    .add_modifier(Modifier::BOLD),
            ))],
            background: palette.panel_bg,
            target: Some(target),
        }
    }
}

pub fn render_mobile_header(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &Snapshot,
    config: &Config,
    hits: &mut Hits,
) {
    if area.is_empty() {
        return;
    }
    let palette = &config.palette;
    buffer.set_style(area, Style::default().bg(palette.panel_bg));
    let button_width = MOBILE_BUTTON_WIDTH.min(area.width);
    let button = Rect::new(
        area.right().saturating_sub(button_width),
        area.y,
        button_width,
        area.height,
    );
    hits.mobile_switch = button;
    let status_width = button.x.saturating_sub(area.x).saturating_sub(1);
    let status = Rect::new(area.x, area.y, status_width, area.height);
    render_header_status(buffer, status, snapshot, config);
    render_header_button(buffer, button, snapshot, config);
}

fn render_header_status(buffer: &mut Buffer, area: Rect, snapshot: &Snapshot, config: &Config) {
    if area.is_empty() {
        return;
    }
    let palette = &config.palette;
    let Some(workspace) = snapshot
        .focused_workspace_id
        .as_deref()
        .and_then(|id| snapshot.workspace(id))
    else {
        put_text(
            buffer,
            area.x,
            area.y,
            area.width,
            " no workspace",
            Style::default().fg(palette.text).bg(palette.panel_bg),
        );
        return;
    };
    let tab_status = compact_tab_status(snapshot, workspace);
    let tab_width = display_width(&tab_status).saturating_add(1).min(area.width);
    let name_width = area.width.saturating_sub(tab_width);
    put_text(
        buffer,
        area.x,
        area.y,
        name_width.min(3),
        &format!(" {} ", status_icon(workspace.agent_status, config.symbols)),
        Style::default()
            .fg(status_color(workspace.agent_status, palette))
            .bg(palette.panel_bg),
    );
    put_text(
        buffer,
        area.x.saturating_add(3),
        area.y,
        name_width.saturating_sub(3),
        &truncate_end(&workspace.label, usize::from(name_width.saturating_sub(4))),
        Style::default()
            .fg(palette.text)
            .bg(palette.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        buffer,
        area.right().saturating_sub(tab_width).saturating_add(1),
        area.y,
        tab_width.saturating_sub(1),
        &tab_status,
        Style::default().fg(palette.overlay1).bg(palette.panel_bg),
    );
    if area.height > 1 {
        render_agent_summary(
            buffer,
            Rect::new(area.x, area.y + 1, area.width, 1),
            snapshot,
            config,
        );
    }
}

fn render_header_button(buffer: &mut Buffer, area: Rect, snapshot: &Snapshot, config: &Config) {
    if area.is_empty() {
        return;
    }
    let palette = &config.palette;
    buffer.set_style(area, Style::default().bg(palette.surface0));
    for y in area.y..area.bottom() {
        put_text(
            buffer,
            area.x,
            y,
            1,
            "│",
            Style::default()
                .fg(palette.surface_dim)
                .bg(palette.surface0),
        );
    }
    let label_y = if area.height > 1 { area.y + 1 } else { area.y };
    let label = "switch";
    let label_width = display_width(label);
    put_text(
        buffer,
        area.x
            .saturating_add(1)
            .saturating_add(area.width.saturating_sub(1 + label_width) / 2),
        label_y,
        area.width.saturating_sub(1),
        label,
        Style::default()
            .fg(palette.text)
            .bg(palette.surface0)
            .add_modifier(Modifier::BOLD),
    );
    if snapshot
        .agents
        .iter()
        .any(|agent| agent.agent_status == Status::Blocked)
    {
        put_text(
            buffer,
            area.right().saturating_sub(1),
            area.y,
            1,
            status_icon(Status::Blocked, config.symbols),
            Style::default().fg(palette.red).bg(palette.surface0),
        );
    }
}

fn compact_tab_status(snapshot: &Snapshot, workspace: &Workspace) -> String {
    let tabs: Vec<_> = snapshot
        .tabs
        .iter()
        .filter(|tab| tab.workspace_id == workspace.workspace_id)
        .collect();
    let active = tabs.iter().position(|tab| tab.focused).unwrap_or(0);
    let label = tabs
        .get(active)
        .map(|tab| tab.label.as_str())
        .unwrap_or("1");
    if tabs.len() <= 1 {
        format!("tab {label}")
    } else {
        format!("tab {label} · {}/{}", active + 1, tabs.len())
    }
}

fn render_agent_summary(buffer: &mut Buffer, area: Rect, snapshot: &Snapshot, config: &Config) {
    let palette = &config.palette;
    let counts = [
        (Status::Blocked, "blocked"),
        (Status::Done, "done"),
        (Status::Working, "working"),
        (Status::Idle, "idle"),
    ]
    .map(|(status, label)| {
        (
            status,
            label,
            snapshot
                .agents
                .iter()
                .filter(|agent| agent.agent_status == status)
                .count(),
        )
    });
    let total = counts.iter().map(|(_, _, count)| count).sum::<usize>();
    let pending = counts[..3].iter().map(|(_, _, count)| count).sum::<usize>();
    if total == 0 || pending == 0 {
        put_text(
            buffer,
            area.x,
            area.y,
            area.width,
            if total == 0 {
                " no agents"
            } else {
                " all idle"
            },
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
        return;
    }
    let mut x = area.x.saturating_add(1);
    let mut shown = 0usize;
    let mut omitted = false;
    for (status, label, count) in counts {
        if count == 0 {
            continue;
        }
        let symbol = match (config.symbols, status) {
            (false, Status::Blocked) => Some("◉"),
            (false, Status::Done) => Some("●"),
            (false, _) => None,
            _ => Some(status_icon(status, config.symbols)),
        };
        let text = symbol.map_or_else(
            || format!("{count} {label}"),
            |symbol| format!("{symbol} {count} {label}"),
        );
        let separator = if shown == 0 { "" } else { " · " };
        let needed = display_width(separator).saturating_add(display_width(&text));
        if area.right().saturating_sub(x) < needed {
            omitted = true;
            break;
        }
        if !separator.is_empty() {
            x = put_segment(
                buffer,
                x,
                area.y,
                area.right(),
                separator,
                Style::default().fg(palette.overlay0).bg(palette.panel_bg),
            );
        }
        let color = if shown == 0 {
            match status {
                Status::Done => palette.blue,
                _ => status_color(status, palette),
            }
        } else {
            palette.overlay1
        };
        x = put_segment(
            buffer,
            x,
            area.y,
            area.right(),
            &text,
            Style::default()
                .fg(color)
                .bg(palette.panel_bg)
                .add_modifier(if shown == 0 {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        );
        shown += 1;
    }
    if omitted && area.right().saturating_sub(x) >= 2 {
        put_text(
            buffer,
            x,
            area.y,
            2,
            " …",
            Style::default().fg(palette.overlay0).bg(palette.panel_bg),
        );
    }
}

/// Machines this connection reaches, by name; empty is the local core.
pub struct Machines<'a> {
    pub names: &'a [String],
    pub current: &'a str,
}

#[allow(clippy::too_many_arguments)]
pub fn render_mobile_switcher(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &Snapshot,
    machines: Machines<'_>,
    config: &Config,
    selected_workspace_id: Option<&str>,
    scroll: &mut usize,
    hits: &mut Hits,
) {
    if area.is_empty() {
        return;
    }
    let palette = &config.palette;
    Clear.render(area, buffer);
    buffer.set_style(area, Style::default().bg(palette.panel_bg));
    hits.mobile_switch = Rect::default();
    let rule = |buffer: &mut Buffer, y: u16| {
        put_text(
            buffer,
            area.x,
            y,
            area.width,
            &"─".repeat(usize::from(area.width)),
            Style::default()
                .fg(palette.surface_dim)
                .bg(palette.panel_bg),
        )
    };
    if area.height <= 2 {
        *scroll = 0;
        rule(buffer, area.y);
        return;
    }
    let header_height = area.height.min(2);
    let close_width = MOBILE_BUTTON_WIDTH.min(area.width);
    let close = Rect::new(
        area.right().saturating_sub(close_width),
        area.y,
        close_width,
        header_height,
    );
    hits.mobile_close = close;
    put_text(
        buffer,
        area.x,
        area.y,
        close.x.saturating_sub(area.x),
        " switch",
        Style::default()
            .fg(palette.text)
            .bg(palette.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    render_close_button(buffer, close, palette);
    let rule_y = area.y + header_height;
    rule(buffer, rule_y);
    let viewport = Rect::new(
        area.x,
        rule_y.saturating_add(1),
        area.width,
        area.height.saturating_sub(header_height + 1),
    );
    if viewport.is_empty() {
        *scroll = 0;
        return;
    }
    let items = mobile_items(
        snapshot,
        &machines,
        config,
        selected_workspace_id,
        viewport.width.saturating_sub(1),
    );
    let total_rows = items.iter().map(|item| item.lines.len()).sum::<usize>();
    let max_scroll = total_rows.saturating_sub(usize::from(viewport.height));
    *scroll = (*scroll).min(max_scroll);
    hits.mobile_max_scroll = max_scroll;
    let content = Rect::new(
        viewport.x + 1,
        viewport.y,
        viewport.width.saturating_sub(1),
        viewport.height,
    );
    if max_scroll > 0 {
        let thumb = (usize::from(viewport.height) * usize::from(viewport.height)
            / total_rows.max(1))
        .max(1);
        let top = *scroll * usize::from(viewport.height) / total_rows.max(1);
        for (row, y) in (viewport.y..viewport.bottom()).enumerate() {
            let on_thumb = row >= top && row < top + thumb;
            put_text(
                buffer,
                viewport.x,
                y,
                1,
                if on_thumb { "┃" } else { "│" },
                Style::default()
                    .fg(if on_thumb {
                        palette.overlay1
                    } else {
                        palette.surface_dim
                    })
                    .bg(palette.panel_bg),
            );
        }
    }
    if content.is_empty() {
        return;
    }
    let viewport_start = *scroll;
    let viewport_end = viewport_start.saturating_add(usize::from(viewport.height));
    let mut document_row = 0usize;
    for item in items {
        let item_start = document_row;
        let item_end = item_start.saturating_add(item.lines.len());
        let visible_start = item_start.max(viewport_start);
        let visible_end = item_end.min(viewport_end);
        if visible_start < visible_end {
            let y = viewport.y + u16::try_from(visible_start - viewport_start).unwrap_or(u16::MAX);
            let height = u16::try_from(visible_end - visible_start).unwrap_or(u16::MAX);
            let rect = Rect::new(content.x, y, content.width, height);
            buffer.set_style(rect, Style::default().bg(item.background));
            for row in visible_start..visible_end {
                Paragraph::new(item.lines[row - item_start].clone()).render(
                    Rect::new(
                        content.x,
                        viewport.y + u16::try_from(row - viewport_start).unwrap_or(u16::MAX),
                        content.width,
                        1,
                    ),
                    buffer,
                );
            }
            if let Some(target) = item.target {
                hits.mobile_targets.push((rect, target));
            }
        }
        document_row = item_end;
    }
}

fn render_close_button(buffer: &mut Buffer, area: Rect, palette: &Palette) {
    if area.is_empty() {
        return;
    }
    buffer.set_style(area, Style::default().bg(palette.surface0));
    for y in area.y..area.bottom() {
        put_text(
            buffer,
            area.x,
            y,
            1,
            "│",
            Style::default()
                .fg(palette.surface_dim)
                .bg(palette.surface0),
        );
    }
    let label_x = area
        .x
        .saturating_add(1)
        .saturating_add(area.width.saturating_sub(6) / 2);
    put_text(
        buffer,
        label_x,
        area.y,
        area.width.saturating_sub(1),
        "close",
        Style::default()
            .fg(palette.overlay1)
            .bg(palette.surface0)
            .add_modifier(Modifier::BOLD),
    );
    if area.height > 1 {
        put_text(
            buffer,
            area.x.saturating_add(area.width / 2),
            area.y + 1,
            1,
            "×",
            Style::default()
                .fg(palette.text)
                .bg(palette.surface0)
                .add_modifier(Modifier::BOLD),
        );
    }
}

fn mobile_items(
    snapshot: &Snapshot,
    machines: &Machines<'_>,
    config: &Config,
    selected_workspace_id: Option<&str>,
    content_width: u16,
) -> Vec<MobileItem> {
    let palette = &config.palette;
    let mut items = Vec::new();
    let machine_label = |name: &str| {
        if name.is_empty() {
            "local".to_owned()
        } else {
            name.to_owned()
        }
    };
    let current = machine_label(machines.current);
    if machines.names.len() > 1 {
        items.push(MobileItem::section("machines", palette));
        for name in machines.names {
            let background = if name == machines.current {
                palette.surface_dim
            } else {
                palette.panel_bg
            };
            items.push(MobileItem {
                lines: vec![Line::from(vec![
                    Span::styled("  ", Style::default().bg(background)),
                    Span::styled(
                        if name == machines.current {
                            "●"
                        } else {
                            "○"
                        },
                        Style::default().fg(palette.green).bg(background),
                    ),
                    Span::styled(
                        format!(" {}", machine_label(name)),
                        Style::default()
                            .fg(palette.text)
                            .bg(background)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])],
                background,
                target: Some(MobileTarget::Machine(name.clone())),
            });
        }
    }
    let agents = ordered_agents(snapshot, config.sort);
    if !agents.is_empty() {
        items.push(MobileItem::section("agents", palette));
        for agent in agents {
            let workspace = agent
                .workspace_id
                .as_deref()
                .and_then(|id| snapshot.workspace(id));
            let agent_label = agent.agent.as_deref().unwrap_or("agent");
            let primary = workspace
                .map(|w| w.label.as_str())
                .unwrap_or(agent.workspace_label.as_str());
            let mut detail = Vec::new();
            if let Some(tab) = agent.tab_id.as_deref().and_then(|id| snapshot.tab(id))
                && (tab.custom_label
                    || snapshot
                        .tabs
                        .iter()
                        .filter(|t| t.workspace_id == tab.workspace_id)
                        .count()
                        > 1)
            {
                detail.push(tab.label.clone());
            }
            detail.push(match agent.agent_status {
                Status::Unknown => "idle".to_owned(),
                status => status.text().to_owned(),
            });
            detail.push(agent_label.to_owned());
            let background = if agent.focused {
                palette.surface_dim
            } else {
                palette.panel_bg
            };
            items.push(MobileItem {
                lines: vec![
                    Line::from(vec![
                        Span::styled("  ", Style::default().bg(background)),
                        Span::styled(
                            status_icon(agent.agent_status, config.symbols),
                            Style::default()
                                .fg(status_color(agent.agent_status, palette))
                                .bg(background),
                        ),
                        Span::styled(" ", Style::default().bg(background)),
                        Span::styled(
                            truncate_end(
                                &format!("{current} · {primary}"),
                                usize::from(content_width.saturating_sub(5)),
                            ),
                            Style::default()
                                .fg(palette.text)
                                .bg(background)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(Span::styled(
                        truncate_end(
                            &format!("  {}", detail.join(" · ")),
                            usize::from(content_width),
                        ),
                        Style::default().fg(palette.overlay0).bg(background),
                    )),
                ],
                background,
                target: Some(MobileTarget::Agent(agent.address.clone())),
            });
        }
    }
    items.push(MobileItem::section("spaces", palette));
    items.push(MobileItem::action(
        "  + new workspace",
        MobileTarget::NewWorkspace,
        palette,
    ));
    for entry in workspace_entries(snapshot, &HashSet::new()) {
        let Some(workspace) = snapshot.workspaces.get(entry.index) else {
            continue;
        };
        let selected = selected_workspace_id == Some(workspace.workspace_id.as_str());
        let background = if selected {
            if palette.surface0 == Color::Reset {
                palette.active_row_bg
            } else {
                palette.surface0
            }
        } else if workspace.focused {
            palette.surface_dim
        } else {
            palette.panel_bg
        };
        let connector = match (entry.indented, entry.last_child) {
            (false, _) => "",
            (true, true) => "└─ ",
            (true, false) => "├─ ",
        };
        let name = if entry.indented && !workspace.custom_label {
            workspace
                .branch
                .as_deref()
                .map(|branch| branch.strip_prefix("worktree/").unwrap_or(branch))
                .unwrap_or(&workspace.label)
        } else {
            &workspace.label
        };
        let branch = workspace.branch.as_deref().unwrap_or("shell");
        let detail_prefix = match (entry.indented, entry.last_child) {
            (false, _) => "  ",
            (true, true) => "       ",
            (true, false) => "  │    ",
        };
        items.push(MobileItem {
            lines: vec![
                Line::from(vec![
                    Span::styled(
                        format!("  {connector}"),
                        Style::default().fg(palette.overlay0).bg(background),
                    ),
                    Span::styled(
                        status_icon(workspace.agent_status, config.symbols),
                        Style::default()
                            .fg(status_color(workspace.agent_status, palette))
                            .bg(background),
                    ),
                    Span::styled(" ", Style::default().bg(background)),
                    Span::styled(
                        truncate_end(
                            &format!("{current} · {name}"),
                            usize::from(content_width.saturating_sub(if entry.indented {
                                8
                            } else {
                                5
                            })),
                        ),
                        Style::default()
                            .fg(palette.text)
                            .bg(background)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(Span::styled(
                    truncate_end(
                        &format!(
                            "{detail_prefix}{branch} · {}",
                            compact_tab_status(snapshot, workspace)
                        ),
                        usize::from(content_width),
                    ),
                    Style::default().fg(palette.overlay0).bg(background),
                )),
            ],
            background,
            target: Some(MobileTarget::Workspace(workspace.workspace_id.clone())),
        });
    }
    if let Some(workspace_id) = snapshot.focused_workspace_id.as_deref() {
        items.push(MobileItem::section("tabs", palette));
        items.push(MobileItem::action(
            "  + new tab",
            MobileTarget::NewTab,
            palette,
        ));
        for (index, tab) in snapshot
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == workspace_id)
            .enumerate()
        {
            let background = if tab.focused {
                palette.surface_dim
            } else {
                palette.panel_bg
            };
            let label = if tab.custom_label {
                format!("{} · {}", index + 1, tab.label)
            } else {
                format!("tab {}", tab.label)
            };
            items.push(MobileItem {
                lines: vec![Line::from(Span::styled(
                    format!(
                        "  {}",
                        truncate_end(&label, usize::from(content_width.saturating_sub(3)))
                    ),
                    Style::default()
                        .fg(palette.text)
                        .bg(background)
                        .add_modifier(Modifier::BOLD),
                ))],
                background,
                target: Some(MobileTarget::Tab(tab.tab_id.clone())),
            });
        }
    }
    items.push(MobileItem::section("menu", palette));
    for (index, (label, _)) in global_menu_items().into_iter().enumerate() {
        items.push(MobileItem {
            lines: vec![Line::from(Span::styled(
                format!("  {label}"),
                Style::default().fg(palette.overlay1).bg(palette.panel_bg),
            ))],
            background: palette.panel_bg,
            target: Some(MobileTarget::Menu(index)),
        });
    }
    items
}
