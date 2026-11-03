use super::*;
use ratatui::style::Style;
use shell::{
    CopyBar,
    chrome::{self, ScrollMetrics, put_text},
    overlays,
};
impl Workspace {
    pub(super) fn draw(
        &mut self,
        screen: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> io::Result<bool> {
        self.renders += 1;
        self.hits = Hits::default();
        let mut resized = false;
        // As Herdr does, the host shows each frame whole instead of as it is written.
        crossterm::queue!(
            screen.backend_mut(),
            crossterm::terminal::BeginSynchronizedUpdate
        )?;
        screen.draw(|frame| {
            if frame.area() != self.area {
                self.geometry(frame.area());
                resized = true;
            }
            self.render_frame(frame)
        })?;
        crossterm::execute!(
            screen.backend_mut(),
            crossterm::terminal::EndSynchronizedUpdate
        )?;
        self.update_window_title()?;
        Ok(resized)
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

    fn render_frame(&mut self, frame: &mut ratatui::Frame) {
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
                paint::terminal(frame, content, &snapshot, surface.copy.as_ref(), &palette);
                if focused {
                    cursor = surface
                        .copy
                        .as_ref()
                        .map(|c| c.cursor)
                        .or(snapshot.cursor)
                        .filter(|(x, y)| *x < content.width && *y < content.height)
                        .map(|(x, y)| (content.x + x, content.y + y));
                }
                if let Some(track) = pane.scrollbar {
                    let (offset, total) = surface.vt.scroll_position();
                    let viewport = u64::from(surface.vt.rows);
                    let max = total.saturating_sub(viewport);
                    let metrics = ScrollMetrics {
                        offset_from_bottom: max.saturating_sub(offset) as usize,
                        max_offset_from_bottom: max as usize,
                        viewport_rows: viewport as usize,
                    };
                    shell::panes::render_scrollbar(
                        frame.buffer_mut(),
                        track,
                        metrics,
                        &palette,
                        focused,
                    );
                }
                if let Some(error) = &surface.error {
                    put_text(
                        frame.buffer_mut(),
                        content.x,
                        content.y,
                        content.width,
                        error,
                        Style::default().fg(palette.overlay0),
                    );
                }
            } else {
                put_text(
                    frame.buffer_mut(),
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
        shell::panes::render_borders(frame.buffer_mut(), &frames, &palette);
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
                frame.buffer_mut(),
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
                frame.buffer_mut(),
                layout.mobile_header,
                &self.snapshot,
                &self.config,
                &mut self.hits,
            );
        } else if self.sidebar_collapsed {
            chrome::render_collapsed_sidebar(
                frame.buffer_mut(),
                layout.sidebar,
                &self.snapshot,
                &self.config,
                self.chrome.selected_workspace_id.as_deref(),
                &mut self.hits,
            );
        } else {
            chrome::render_sidebar(
                frame.buffer_mut(),
                layout.sidebar,
                &self.snapshot,
                &self.config,
                &mut self.chrome,
                &mut self.hits,
            );
        }
        chrome::render_tab_bar(
            frame.buffer_mut(),
            layout.tab_bar,
            &self.snapshot,
            &self.config,
            &mut self.chrome,
            &mut self.hits,
        );
        let copy = (self.mode == Mode::Copy).then(|| self.copy_bar()).flatten();
        if let Some(bar) = chrome::render_mode_bar(
            frame.buffer_mut(),
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
            let area = frame.area();
            shell::mobile::render_mobile_switcher(
                frame.buffer_mut(),
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
                    frame.buffer_mut(),
                    self.hits.global_launcher,
                    *highlighted,
                    &palette,
                ),
                overlay => overlays::render(
                    frame.buffer_mut(),
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
        self.render_notification(frame.buffer_mut(), layout.pane_surface);
        if let Some(toast) = &self.toast {
            overlays::render_toast(
                frame.buffer_mut(),
                layout.pane_surface,
                &toast.text,
                toast.error,
                &palette,
            );
        }
        if let Some(position) = cursor {
            frame.set_cursor_position(position);
        }
    }
}
