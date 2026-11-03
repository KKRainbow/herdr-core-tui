//! Herdr's agent notifications: a delay, a recheck, sound, and delivery inside the
//! TUI, through the host terminal, or through the desktop. Adapted from Herdr
//! `src/client/shell/notification_policy.rs`, `src/client/shell/notifications.rs`,
//! and `src/app/actions.rs` at ee5bfb5041bb3e4a49d591c72308aae3c81ecdde.
use super::*;
use herdr_client_lib::{model::Alert, view::Identity};
use ratatui::{
    buffer::Buffer,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};
use std::collections::VecDeque;

const MAX_QUEUED_NOTIFICATIONS: usize = 8;

#[derive(Clone, Debug)]
pub(super) struct Notification {
    alert: Alert,
    identity: Identity,
    /// The terminal it is about, as this connection addresses it.
    pub(super) address: String,
    title: String,
    body: String,
    deadline: Instant,
}

#[derive(Default)]
pub(super) struct Notifications {
    pending: Vec<Notification>,
    pub(super) visible: Option<Notification>,
    queued: VecDeque<Notification>,
}

impl Notifications {
    /// A card covers part of the pane surface.
    pub(super) fn card_shown(&self) -> bool {
        self.visible.is_some()
    }
    pub(super) fn active(&self) -> bool {
        self.visible.is_some() || !self.pending.is_empty()
    }
}

fn duration(alert: Alert) -> Duration {
    Duration::from_secs(match alert {
        Alert::Blocked => 8,
        Alert::Done => 5,
    })
}

impl Workspace {
    /// Starts Herdr's notification for an alert; it is delivered after `ui.toast.delay_seconds`.
    pub(super) fn notify(&mut self, identity: Identity, address: &str, alert: Alert, now: Instant) {
        let record = self.session.terminals.get(&identity);
        let agent = record
            .and_then(herdr_client_lib::model::program_label)
            .unwrap_or("agent");
        let title = format!(
            "{agent} {}",
            match alert {
                Alert::Blocked => "needs attention",
                Alert::Done => "finished",
            }
        );
        let body = self.notification_context(address);
        let state = &mut self.notifications;
        state.pending.retain(|n| n.address != address);
        state.queued.retain(|n| n.address != address);
        if state.visible.as_ref().is_some_and(|n| n.address == address) {
            state.visible = None;
            Self::promote(state, now);
        }
        let delay = Duration::from_secs(self.preferences.ui.toast.delay_seconds);
        state.pending.push(Notification {
            alert,
            identity,
            address: address.to_owned(),
            title,
            body,
            deadline: now + delay,
        });
        self.notification_target = Some(address.to_owned());
        self.tick_notifications(now);
    }

    /// `workspace · number`, plus the tab when its space has several, as Herdr writes it.
    fn notification_context(&self, address: &str) -> String {
        let Some(agent) = self.snapshot.agents.iter().find(|a| a.address == address) else {
            return String::new();
        };
        let Some(space) = agent
            .workspace_id
            .as_deref()
            .and_then(|id| self.snapshot.workspace(id))
        else {
            return agent.workspace_label.clone();
        };
        let mut context = format!("{} · {}", space.label, space.number);
        let tabs: Vec<_> = self
            .snapshot
            .tabs
            .iter()
            .filter(|t| t.workspace_id == space.workspace_id)
            .collect();
        if tabs.len() > 1
            && let Some(tab) = tabs
                .iter()
                .find(|t| Some(&t.tab_id) == agent.tab_id.as_ref())
        {
            context.push_str(&format!(" · {}", tab.label));
        }
        context
    }

    fn promote(state: &mut Notifications, now: Instant) {
        if let Some(mut next) = state.queued.pop_front() {
            next.deadline = now + duration(next.alert);
            state.visible = Some(next);
        }
    }

    /// Delivers notifications whose delay passed and retires expired cards. Returns
    /// whether the screen needs a repaint.
    pub(super) fn tick_notifications(&mut self, now: Instant) -> bool {
        let mut repaint = false;
        if self
            .notifications
            .visible
            .as_ref()
            .is_some_and(|n| now >= n.deadline)
        {
            self.notifications.visible = None;
            Self::promote(&mut self.notifications, now);
            repaint = true;
        }
        let pending = std::mem::take(&mut self.notifications.pending);
        for notification in pending {
            if notification.deadline > now {
                self.notifications.pending.push(notification);
                continue;
            }
            // Still news when delivered: a finish not yet seen, a request still open.
            let word = self
                .session
                .terminals
                .get(&notification.identity)
                .and_then(|r| r.word());
            let current = match notification.alert {
                Alert::Done => {
                    self.attention.unseen(&notification.identity)
                        && !matches!(word, Some("working" | "blocked" | "error"))
                }
                Alert::Blocked => matches!(word, Some("blocked" | "error")),
            };
            if !current {
                continue;
            }
            let target_active = self.panes.iter().any(|p| {
                p.terminal
                    .as_deref()
                    .is_some_and(|t| qualify(t, &self.machine) == notification.address)
            });
            let suppress_external = target_active && self.focused;
            if self.preferences.sound.enabled
                && !(notification.alert == Alert::Done && suppress_external)
            {
                let sound = crate::sound::for_alert(notification.alert);
                crate::sound::play(sound, self.preferences.sound.path(sound));
            }
            let body = Some(notification.body.as_str()).filter(|b| !b.is_empty());
            let delivered = match self.preferences.ui.toast.delivery.as_str() {
                "herdr" if !target_active => {
                    let state = &mut self.notifications;
                    if state.visible.is_none() {
                        let mut card = notification;
                        card.deadline = now + duration(card.alert);
                        state.visible = Some(card);
                    } else {
                        if state.queued.len() == MAX_QUEUED_NOTIFICATIONS {
                            state.queued.pop_front();
                        }
                        state.queued.push_back(notification);
                    }
                    repaint = true;
                    Ok(true)
                }
                "terminal" if !suppress_external => {
                    crate::notify::show_terminal_notification(&notification.title, body)
                }
                "system" if !suppress_external => {
                    crate::notify::show_desktop_notification(&notification.title, body)
                }
                _ => Ok(false),
            };
            if let Err(error) = delivered {
                self.toast(error.to_string(), true);
                repaint = true;
            }
        }
        repaint
    }

    /// Herdr's open notification target: the visible card's agent, else the latest alert.
    pub(super) fn open_notification(&mut self) {
        let address = match self.notifications.visible.take() {
            Some(card) => {
                Self::promote(&mut self.notifications, Instant::now());
                Some(card.address)
            }
            None => self.notification_target.clone(),
        };
        if let Some(address) = address {
            self.focus_agent(&address);
        }
    }

    pub(super) fn render_notification(&self, buffer: &mut Buffer, area: Rect) {
        let Some(card) = &self.notifications.visible else {
            return;
        };
        let palette = &self.config.palette;
        let dot = match card.alert {
            Alert::Blocked => palette.red,
            Alert::Done => palette.blue,
        };
        render_card(
            buffer,
            area,
            &card.title,
            &card.body,
            &self.preferences.ui.toast.herdr.position,
            dot,
            palette,
        );
    }
}

fn render_card(
    buffer: &mut Buffer,
    area: Rect,
    title: &str,
    body: &str,
    position: &str,
    dot_color: Color,
    palette: &shell::palette::Palette,
) -> Rect {
    if area.is_empty() {
        return Rect::default();
    }
    let content_width = unicode_width::UnicodeWidthStr::width(title)
        .max(unicode_width::UnicodeWidthStr::width(body))
        .saturating_add(6);
    let width = u16::try_from(content_width)
        .unwrap_or(u16::MAX)
        .min(area.width);
    let height: u16 = if body.is_empty() { 3 } else { 4 }.min(area.height);
    let left = position.ends_with("left");
    let top = position.starts_with("top");
    let x = if left {
        area.x
    } else {
        area.right().saturating_sub(width)
    };
    let max_y = area.bottom().saturating_sub(height).max(area.y);
    let y = if top {
        area.y
    } else {
        area.bottom().saturating_sub(height)
    }
    .clamp(area.y, max_y);
    let rect = Rect::new(x, y, width, height);
    Clear.render(rect, buffer);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.overlay0))
        .style(Style::default().bg(palette.panel_bg));
    let inner = block.inner(rect);
    block.render(rect, buffer);
    Paragraph::new(Line::from(vec![
        Span::styled("●", Style::default().fg(dot_color)),
        Span::raw(" "),
        Span::styled(
            title,
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .render(Rect::new(inner.x, inner.y, inner.width, 1), buffer);
    if !body.is_empty() && inner.height > 1 {
        Paragraph::new(Line::from(Span::styled(
            body,
            Style::default().fg(palette.overlay0),
        )))
        .render(
            Rect::new(
                inner.x.saturating_add(2),
                inner.y + 1,
                inner.width.saturating_sub(2),
                1,
            ),
            buffer,
        );
    }
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_sit_in_the_configured_corner() {
        let palette = shell::palette::Palette::default();
        let area = Rect::new(0, 0, 40, 10);
        let mut buffer = Buffer::empty(area);
        let rect = render_card(
            &mut buffer,
            area,
            "pi finished",
            "api · 1",
            "bottom-right",
            palette.blue,
            &palette,
        );
        assert_eq!(rect, Rect::new(23, 6, 17, 4));
        let rect = render_card(
            &mut buffer,
            area,
            "pi finished",
            "",
            "top-left",
            palette.blue,
            &palette,
        );
        assert_eq!(rect, Rect::new(0, 0, 17, 3));
    }
}
