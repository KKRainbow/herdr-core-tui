use super::*;
use ratatui::{
    buffer::{Buffer, Cell},
    style::Style,
};
use shell::{
    CopyBar,
    chrome::{self, ScrollMetrics, put_text},
    overlays,
};
use std::sync::Arc;
impl Workspace {
    pub(super) fn draw(
        &mut self,
        screen: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> io::Result<bool> {
        self.renders += 1;
        self.hits = Hits::default();
        let size = screen.size()?;
        let area = Rect::new(0, 0, size.width, size.height);
        // The host can resize while streams resize; redraw at the size the frame used.
        let resized = area != self.area;
        if resized {
            self.geometry(area);
        }
        if self.presented.area != area {
            screen.clear()?;
            self.presented = Buffer::empty(area);
        }
        let mut next = Buffer::empty(area);
        let cursor = self.render_frame(&mut next);
        let updates = self.presented.diff(&next);
        present(screen, updates.into_iter(), cursor)?;
        self.presented = next;
        self.update_window_title()?;
        Ok(resized)
    }

    /// Herdr's retained surface: when only visible terminals printed, send just the
    /// rows they changed. Returns false when the frame needs a full redraw instead.
    pub(super) fn draw_output(
        &mut self,
        screen: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> io::Result<bool> {
        let size = screen.size()?;
        let area = Rect::new(0, 0, size.width, size.height);
        let covered = self.overlay.is_some()
            || self.toast.is_some()
            || self.copy_search.is_some()
            || self.drag.is_some()
            || self.mode != Mode::Terminal
            || self.notifications.card_shown();
        if covered || area != self.area || self.presented.area != area {
            return Ok(false);
        }
        let mut frames = Vec::with_capacity(self.panes.len());
        for (index, pane) in self.panes.iter().enumerate() {
            let Some(surface) = self.surfaces.get_mut(&pane.id) else {
                continue;
            };
            if surface.copy.is_some() || surface.error.is_some() {
                return Ok(false);
            }
            let frame = surface.vt.frame();
            if frame.rows.len() != surface.painted.len() {
                return Ok(false);
            }
            frames.push((index, frame));
        }
        self.renders += 1;
        self.retained_renders += 1;
        let palette = self.config.palette.clone();
        let focused_id = self.active_id();
        let mut updates = Vec::new();
        let mut cursor = None;
        for (index, frame) in frames {
            let pane = &self.panes[index];
            let Some(surface) = self.surfaces.get_mut(&pane.id) else {
                continue;
            };
            let focused = self.peek.is_some() || focused_id.as_deref() == Some(&pane.id);
            let content = pane.content.intersection(area);
            for (y, row) in frame.rows.iter().take(content.height as usize).enumerate() {
                if Arc::ptr_eq(row, &surface.painted[y]) {
                    continue;
                }
                let rect = Rect::new(content.x, content.y + y as u16, content.width, 1);
                let mut after = Buffer::empty(rect);
                paint::row(&mut after, rect, 0, row, None);
                retain(&mut self.presented, &after, &mut updates);
            }
            if let Some(track) = pane.scrollbar {
                let mut after = Buffer::empty(track);
                shell::panes::render_scrollbar(
                    &mut after,
                    track,
                    scroll_metrics(&surface.vt),
                    &palette,
                    focused,
                );
                retain(&mut self.presented, &after, &mut updates);
            }
            if focused {
                cursor = pane_cursor(None, &frame, content);
            }
            surface.painted = frame.rows;
        }
        present(
            screen,
            updates.iter().map(|(x, y, cell)| (*x, *y, cell)),
            cursor,
        )?;
        Ok(true)
    }

    /// Herdr's `ui.window_title` on the outer terminal, written only when it changes.
    pub(super) fn update_window_title(&mut self) -> io::Result<()> {
        use crate::window_title::{Token, hostname, parse, render};
        let Ok(Some(parts)) = parse(&self.preferences.ui.window_title) else {
            return Ok(());
        };
        let view = self.view();
        let pane = view
            .focused
            .as_deref()
            .and_then(|id| self.snapshot.pane(id));
        let title = render(&parts, |token| match token {
            Token::Hostname if self.machine.is_empty() => Some(hostname()),
            Token::Hostname => Some(self.machine.clone()),
            Token::Workspace => view
                .active_space
                .as_deref()
                .and_then(|id| self.snapshot.workspace(id))
                .map(|w| w.label.clone()),
            Token::Tab => view
                .active_tab
                .as_deref()
                .and_then(|id| self.snapshot.tab(id))
                .map(|t| t.label.clone()),
            Token::Pane => pane.and_then(|p| p.label.clone()),
            Token::TerminalTitle => pane.and_then(|p| p.terminal.as_deref()).and_then(|local| {
                let address = qualify(local, &self.machine);
                self.session
                    .terminals
                    .values()
                    .find(|r| r.address == address)
                    .map(|r| r.title.clone())
            }),
        });
        if title.is_some() && title != self.window_title {
            crossterm::execute!(
                io::stdout(),
                crossterm::terminal::SetTitle(title.as_deref().unwrap_or_default())
            )?;
            self.window_title = title;
        }
        Ok(())
    }

    fn copy_bar(&self) -> Option<CopyBar> {
        let surface = self.active_id().and_then(|id| self.surfaces.get(&id))?;
        let copy = surface.copy.as_ref();
        Some(CopyBar {
            search_prompt: self
                .copy_search
                .as_ref()
                .map(|search| search.query.as_str().to_owned()),
            query: copy.map(|c| c.query.clone()).unwrap_or_default(),
            matched: copy.is_some_and(|c| !c.query.is_empty() && c.selecting),
            selecting: copy.is_some_and(|c| c.selecting),
        })
    }

    /// Draws the whole screen into `buffer` and returns where the cursor goes.
    fn render_frame(&mut self, buffer: &mut Buffer) -> Option<(u16, u16)> {
        let palette = self.config.palette.clone();
        (
            self.chrome.dragged_workspace_id,
            self.chrome.workspace_drop_row,
            self.chrome.tab_drop_index,
        ) = match &self.drag {
            Some(Drag::Workspace { source, target }) => (
                Some(source.clone()),
                target.as_ref().map(|(_, row)| *row),
                None,
            ),
            Some(Drag::Tab { insert, .. }) => (None, None, *insert),
            _ => (None, None, None),
        };
        let view = self.view();
        let mut cursor = None;
        for pane in &self.panes {
            let focused = self.peek.is_some() || view.focused.as_deref() == Some(&pane.id);
            let content = pane.content;
            if let Some(surface) = self.surfaces.get_mut(&pane.id) {
                let snapshot = match &surface.copy {
                    Some(copy) => copy.frame.clone(),
                    None => surface.vt.frame(),
                };
                paint::terminal(buffer, content, &snapshot, surface.copy.as_ref(), &palette);
                if focused {
                    cursor = pane_cursor(surface.copy.as_ref(), &snapshot, content);
                }
                if let Some(track) = pane.scrollbar {
                    let metrics = scroll_metrics(&surface.vt);
                    shell::panes::render_scrollbar(buffer, track, metrics, &palette, focused);
                }
                // A copy-mode frame is not the terminal's, so the next output redraws fully.
                surface.painted = if surface.copy.is_some() {
                    Vec::new()
                } else {
                    snapshot.rows
                };
                if let Some(error) = &surface.error {
                    put_text(
                        buffer,
                        content.x,
                        content.y,
                        content.width,
                        error,
                        Style::default().fg(palette.overlay0),
                    );
                }
            } else {
                put_text(
                    buffer,
                    content.x,
                    content.y,
                    content.width,
                    &format!("Unsupported app pane: {}", pane.description),
                    Style::default().fg(palette.overlay0),
                );
            }
        }
        let frames: Vec<_> = self
            .panes
            .iter()
            .map(|pane| shell::panes::Frame {
                rect: pane.area,
                borders: pane.borders,
                focused: self.peek.is_some() || view.focused.as_deref() == Some(&pane.id),
                label: self
                    .snapshot
                    .pane(&pane.id)
                    .and_then(|p| p.label.clone())
                    .or_else(|| {
                        // Herdr's `show_agent_labels_on_pane_borders`: the agent's name when unnamed.
                        self.preferences
                            .ui
                            .show_agent_labels_on_pane_borders
                            .then(|| self.snapshot.agents.iter().find(|a| a.pane_id == pane.id))
                            .flatten()
                            .and_then(|a| a.agent.clone())
                    }),
            })
            .collect();
        shell::panes::render_borders(buffer, &frames, &palette);
        if self.panes.is_empty() {
            let surface = self.layout.pane_surface;
            let message = if self.session.document_ref(&self.machine).is_err() {
                format!(
                    "{}: unavailable. Select a connected machine.",
                    if self.machine.is_empty() {
                        "Local"
                    } else {
                        &self.machine
                    }
                )
            } else {
                "no workspace · click new or press prefix+shift+n".into()
            };
            put_text(
                buffer,
                surface.x + 1,
                surface.y,
                surface.width.saturating_sub(1),
                &message,
                Style::default().fg(palette.overlay0),
            );
        }
        let layout = self.layout;
        if !layout.mobile_header.is_empty() {
            shell::mobile::render_mobile_header(
                buffer,
                layout.mobile_header,
                &self.snapshot,
                &self.config,
                &mut self.hits,
            );
        } else if self.sidebar_collapsed {
            chrome::render_collapsed_sidebar(
                buffer,
                layout.sidebar,
                &self.snapshot,
                &self.config,
                self.chrome.selected_workspace_id.as_deref(),
                &mut self.hits,
            );
        } else {
            chrome::render_sidebar(
                buffer,
                layout.sidebar,
                &self.snapshot,
                &self.config,
                &mut self.chrome,
                &mut self.hits,
            );
        }
        chrome::render_tab_bar(
            buffer,
            layout.tab_bar,
            &self.snapshot,
            &self.config,
            &mut self.chrome,
            &mut self.hits,
        );
        let copy = (self.mode == Mode::Copy).then(|| self.copy_bar()).flatten();
        if let Some(bar) = chrome::render_mode_bar(
            buffer,
            layout.pane_surface,
            self.mode,
            copy.as_ref(),
            None,
            &palette,
        ) {
            if cursor.is_some_and(|(_, y)| y == bar.y) {
                cursor = None;
            }
            if let Some(search) = &self.copy_search {
                let (_, offset) = search.query.viewport(bar.width.saturating_sub(8));
                cursor = Some((bar.x + 8 + offset, bar.y));
            }
        }
        if self.mobile_switcher_open() {
            let area = buffer.area;
            shell::mobile::render_mobile_switcher(
                buffer,
                area,
                &self.snapshot,
                shell::mobile::Machines {
                    names: &self.machines,
                    current: &self.machine,
                },
                &self.config,
                self.chrome.selected_workspace_id.as_deref(),
                &mut self.chrome.mobile_scroll,
                &mut self.hits,
            );
            cursor = None;
        }
        self.overlay_render = OverlayRender::default();
        if let Some(overlay) = &self.overlay {
            let machines = self.machines.clone();
            let terminals = &self.session.terminals;
            let cwd = |address: &str| {
                terminals
                    .values()
                    .find(|r| r.address == address)
                    .and_then(snapshot::cwd)
                    .map(str::to_owned)
            };
            let rendered = match overlay {
                Overlay::GlobalMenu { highlighted } => overlays::render_global_menu(
                    buffer,
                    self.hits.global_launcher,
                    *highlighted,
                    &palette,
                ),
                overlay => overlays::render(
                    buffer,
                    overlay,
                    &overlays::Context {
                        snapshot: &self.snapshot,
                        keys: &self.keys,
                        palette: &palette,
                        symbols: self.config.symbols,
                        machines: &machines,
                        machine: &self.machine,
                        cwd: &cwd,
                    },
                ),
            };
            self.overlay_render = rendered.unwrap_or_default();
            cursor = self.overlay_render.cursor.map(|p| (p.x, p.y)).or(cursor);
        }
        self.render_notification(buffer, layout.pane_surface);
        if let Some(toast) = &self.toast {
            overlays::render_toast(
                buffer,
                layout.pane_surface,
                &toast.text,
                toast.error,
                &palette,
            );
        }
        cursor
    }
}

/// The focused pane's cursor on the host, inside its content.
fn pane_cursor(copy: Option<&Copy>, frame: &crate::vt::Frame, content: Rect) -> Option<(u16, u16)> {
    copy.map(|c| c.cursor)
        .or(frame.cursor)
        .filter(|(x, y)| *x < content.width && *y < content.height)
        .map(|(x, y)| (content.x + x, content.y + y))
}
fn scroll_metrics(vt: &Vt) -> ScrollMetrics {
    let (offset, total) = vt.scroll_position();
    let viewport = u64::from(vt.rows);
    let max = total.saturating_sub(viewport);
    ScrollMetrics {
        offset_from_bottom: max.saturating_sub(offset) as usize,
        max_offset_from_bottom: max as usize,
        viewport_rows: viewport as usize,
    }
}
/// Copies `after` over the same area of `presented`, collecting the cells the host must change.
fn retain(presented: &mut Buffer, after: &Buffer, updates: &mut Vec<(u16, u16, Cell)>) {
    let area = after.area.intersection(presented.area);
    let mut before = Buffer::empty(area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            before[(x, y)] = presented[(x, y)].clone();
        }
    }
    updates.extend(
        before
            .diff(after)
            .into_iter()
            .map(|(x, y, cell)| (x, y, cell.clone())),
    );
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            presented[(x, y)] = after[(x, y)].clone();
        }
    }
}
/// Writes changed cells and the cursor as one synchronized host frame, as Herdr does.
fn present<'a>(
    screen: &mut Terminal<CrosstermBackend<io::Stdout>>,
    updates: impl Iterator<Item = (u16, u16, &'a Cell)>,
    cursor: Option<(u16, u16)>,
) -> io::Result<()> {
    use ratatui::backend::Backend;
    let backend = screen.backend_mut();
    crossterm::queue!(backend, crossterm::terminal::BeginSynchronizedUpdate)?;
    backend.draw(updates)?;
    match cursor {
        Some(position) => {
            backend.show_cursor()?;
            backend.set_cursor_position(position)?;
        }
        None => backend.hide_cursor()?,
    }
    crossterm::queue!(backend, crossterm::terminal::EndSynchronizedUpdate)?;
    Backend::flush(backend)
}
