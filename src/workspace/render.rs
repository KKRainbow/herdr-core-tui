use super::*;
impl Workspace {
    pub(super) fn draw(
        &mut self,
        screen: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> io::Result<()> {
        self.renders += 1;
        let view = self.view();
        let doc = self.document();
        let sidebar = self.preferences.sidebar_width;
        self.hits.clear();
        screen.draw(|frame| self.render_frame(frame, &view, &doc, sidebar))?;
        Ok(())
    }
    fn render_frame(
        &mut self,
        frame: &mut ratatui::Frame,
        view: &View,
        doc: &Option<herdr_client_lib::document::Document>,
        sidebar: u16,
    ) {
        let area = frame.area();
        let width = sidebar.min(area.width / 3);
        let sidebar_area = Rect::new(0, 0, width, area.height.saturating_sub(1));
        frame.render_widget(
            Block::default().borders(Borders::RIGHT).title(format!(
                " {} · {} ",
                if self.machine.is_empty() {
                    "Local"
                } else {
                    &self.machine
                },
                self.session_name
            )),
            sidebar_area,
        );
        let mut y = 1;
        if let Some(doc) = &doc {
            for space in doc.value()["spaces"].as_array().into_iter().flatten() {
                if y >= area.height.saturating_sub(4) {
                    break;
                }
                let id = space["id"].as_str().unwrap_or("");
                let text = format!(
                    "{} {}",
                    if view.active_space.as_deref() == Some(id) {
                        "▸"
                    } else {
                        " "
                    },
                    space["name"].as_str().unwrap_or("Workspace")
                );
                let row = Rect::new(0, y, width.saturating_sub(1), 1);
                frame.render_widget(Paragraph::new(text), row);
                self.hits.push((row, Hit::Space(id.into())));
                y += 1;
            }
        }
        y += 1;
        if y < area.height.saturating_sub(2) {
            frame.render_widget(
                Paragraph::new(" Terminals").style(Style::default().fg(Color::Cyan)),
                Rect::new(0, y, width.saturating_sub(1), 1),
            );
            y += 1;
        }
        for (identity, record) in &self.session.terminals {
            if y >= area.height.saturating_sub(2) {
                break;
            }
            let Some(address) = record["address"].as_str() else {
                continue;
            };
            let status = record["status"]["state"]
                .as_str()
                .or_else(|| record["status"].as_str());
            if record["state"] != "live" && status.is_none() && record["reported"] != true {
                continue;
            }
            let unseen = self
                .attention
                .entries
                .iter()
                .any(|entry| &entry.identity == identity && entry.unseen);
            let text = format!(
                "{}{} {}",
                if unseen { "*" } else { "" },
                address,
                status.unwrap_or_else(|| record["state"].as_str().unwrap_or("unknown"))
            );
            let row = Rect::new(0, y, width.saturating_sub(1), 1);
            frame.render_widget(Paragraph::new(text), row);
            self.hits.push((row, Hit::Agent(address.into())));
            y += 1;
        }
        let mut x = width;
        if let Some(space) = doc
            .as_ref()
            .and_then(|d| view.active_space.as_deref().and_then(|id| d.node(id)))
        {
            let tabs = space["tabs"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            let titles: Vec<_> = tabs
                .iter()
                .map(|tab| {
                    format!(
                        " {} ",
                        tab["name"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("Terminal")
                    )
                })
                .collect();
            let widths: Vec<_> = titles
                .iter()
                .map(|title| {
                    unicode_width::UnicodeWidthStr::width(title.as_str()).min(u16::MAX as usize)
                        as u16
                })
                .collect();
            let active = tabs
                .iter()
                .position(|tab| tab["id"].as_str() == view.active_tab.as_deref())
                .unwrap_or(0);
            let start = tab_window_start(&widths, active, area.width.saturating_sub(width));
            for (tab, title) in tabs.iter().zip(titles.iter()).skip(start) {
                if x >= area.width {
                    break;
                }
                let id = tab["id"].as_str().unwrap_or("");
                let w = (unicode_width::UnicodeWidthStr::width(title.as_str())
                    .min(u16::MAX as usize) as u16)
                    .min(area.width.saturating_sub(x));
                let rect = Rect::new(x, 0, w, 1);
                frame.render_widget(
                    Paragraph::new(title.as_str()).style(Style::default().fg(
                        if view.active_tab.as_deref() == Some(id) {
                            Color::Yellow
                        } else {
                            Color::Gray
                        },
                    )),
                    rect,
                );
                self.hits.push((rect, Hit::Tab(id.into())));
                x += w;
            }
        }
        for pane in &self.panes {
            let active = self.peek.is_some() || view.focused.as_deref() == Some(&pane.id);
            let title = pane.title();
            let block = Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(if active {
                    Color::Cyan
                } else {
                    Color::DarkGray
                }));
            frame.render_widget(block, pane.area);
            let area = inner(pane.area);
            self.hits.push((pane.area, Hit::Pane(pane.id.clone())));
            if let Some(surface) = self.surfaces.get_mut(&pane.id) {
                if let Some(copy) = &surface.copy {
                    paint::terminal(frame, area, &copy.frame, active, Some(copy));
                } else {
                    paint::terminal(frame, area, &surface.vt.frame(), active, None);
                }
                if let Some(error) = &surface.error {
                    frame.render_widget(Paragraph::new(error.as_str()), area);
                }
            } else {
                frame.render_widget(
                    Paragraph::new(format!("Unsupported app pane: {}", pane.description)),
                    area,
                );
            }
        }
        if self.panes.is_empty() {
            frame.render_widget(
                Paragraph::new(if doc.is_none() {
                    "Workspace unavailable; waiting for its machine"
                } else {
                    "Ctrl+B s: new space   Ctrl+B ?: help"
                }),
                Rect::new(width + 1, 2, area.width.saturating_sub(width + 1), 1),
            );
        }
        let status = if self.prefix {
            "Command: s space · t tab · %/\" split · p pane · n tab · z zoom · x close · [ copy · : command"
        } else if self.notice.is_empty() {
            "Ctrl+B commands · Ctrl+Q quit · Shift+drag select"
        } else {
            &self.notice
        };
        frame.render_widget(
            Paragraph::new(status),
            Rect::new(0, area.height.saturating_sub(1), area.width, 1),
        );
        if let Some(dialog) = &self.dialog {
            crate::dialog::render(frame, dialog);
        }
    }
}

fn tab_window_start(widths: &[u16], active: usize, available: u16) -> usize {
    let active = active.min(widths.len().saturating_sub(1));
    let mut start = active;
    let mut used = widths.get(active).copied().unwrap_or(0).min(available);
    while start > 0 && u32::from(used) + u32::from(widths[start - 1]) <= u32::from(available) {
        start -= 1;
        used += widths[start];
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflow_window_keeps_selected_tab_visible() {
        assert_eq!(tab_window_start(&[8, 8, 8, 8], 3, 20), 2);
        assert_eq!(tab_window_start(&[8, 8, 8, 8], 0, 20), 0);
        assert_eq!(tab_window_start(&[8, 80, 8], 1, 20), 1);
        assert_eq!(tab_window_start(&[8, 8, 8], 2, 24), 0);
        assert_eq!(tab_window_start(&[], 0, 0), 0);
        assert_eq!(tab_window_start(&[8, 8], 1, 0), 1);
    }
}
