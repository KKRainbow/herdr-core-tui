//! Optional host-owned message entry. Core and native terminal input stay separate.
use crate::{editor::TextEditor, shell::palette::Palette};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};
use std::{collections::VecDeque, future::Future, pin::Pin};
use tokio::sync::mpsc;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

type Submission = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;
const HISTORY_BYTES: usize = 256 * 1024;
const MESSAGE_BYTES: usize = 64 * 1024;

/// A message supplied by the embedding host, separate from native terminal output.
pub struct ComposerMessage {
    pub title: String,
    pub text: String,
}

/// A host supplies submissions, receipts and messages; the TUI owns their display.
pub struct Composer {
    title: String,
    submit: Box<dyn Fn(u64, String) -> Submission + Send>,
    editor: TextEditor,
    pub(crate) visible: bool,
    revision: u64,
    pending: Option<Submission>,
    status: String,
    messages: Option<mpsc::Receiver<ComposerMessage>>,
    history: VecDeque<ComposerMessage>,
    history_bytes: usize,
    history_enabled: bool,
    history_scroll: usize,
    history_page: usize,
}

impl Composer {
    /// `id` stays the same for an unchanged retry, and changes after an edit or
    /// accepted submission. Scope it to this viewer in the host's message ID.
    pub fn new<F, Fut>(title: impl Into<String>, submit: F) -> Self
    where
        F: Fn(u64, String) -> Fut + Send + 'static,
        Fut: Future<Output = Result<String, String>> + Send + 'static,
    {
        Self {
            title: title.into(),
            submit: Box::new(move |id, text| Box::pin(submit(id, text))),
            editor: TextEditor::multiline(),
            visible: false,
            revision: 0,
            pending: None,
            status: String::new(),
            messages: None,
            history: VecDeque::new(),
            history_bytes: 0,
            history_enabled: false,
            history_scroll: 0,
            history_page: 1,
        }
    }

    /// The host retains durable history and chooses which messages to supply.
    /// This viewer retains up to 100 messages and 256 KiB, with 64 KiB per message.
    pub fn with_messages(mut self, messages: mpsc::Receiver<ComposerMessage>) -> Self {
        self.messages = Some(messages);
        self.history_enabled = true;
        self
    }

    pub(crate) fn open(&mut self) {
        self.visible = true;
    }

    pub(crate) fn handle(&mut self, event: &Event) -> bool {
        if !self.visible {
            return false;
        }
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Release => {}
            Event::Key(key) if key.code == KeyCode::Esc => self.visible = false,
            Event::Key(key) if key.code == KeyCode::PageUp => {
                self.history_scroll = self.history_scroll.saturating_add(self.history_page);
            }
            Event::Key(key) if key.code == KeyCode::PageDown => {
                self.history_scroll = self.history_scroll.saturating_sub(self.history_page);
            }
            Event::Key(key) if self.pending.is_none() => {
                if key.code == KeyCode::Char('s') && key.modifiers == KeyModifiers::CONTROL {
                    if self.editor.trim().is_empty() {
                        self.status = "Message is empty".into();
                    } else {
                        self.pending = Some((self.submit)(self.revision, self.editor.to_string()));
                        self.status = "Submitting to host...".into();
                    }
                } else if self.editor.handle_key(key) == Some(true) {
                    self.revision += 1;
                    self.status.clear();
                }
            }
            Event::Paste(text) if self.pending.is_none() => {
                if self.editor.insert(text) {
                    self.revision += 1;
                    self.status.clear();
                }
            }
            Event::Key(_) | Event::Paste(_) | Event::Mouse(_) => {}
            _ => return false,
        }
        true
    }

    pub(crate) async fn next(&mut self) -> Option<Result<String, String>> {
        loop {
            tokio::select! {
                result = async {
                    match &mut self.pending {
                        Some(pending) => pending.await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.pending = None;
                    match &result {
                        Ok(receipt) => {
                            self.editor.clear();
                            self.revision += 1;
                            self.status = receipt.clone();
                        }
                        Err(error) => self.status = error.clone(),
                    }
                    return Some(result);
                }
                message = async {
                    match &mut self.messages {
                        Some(messages) => messages.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Some(mut message) = message {
                        truncate(&mut message.title, 256);
                        truncate(&mut message.text, MESSAGE_BYTES - message.title.len());
                        self.history_bytes += message.title.len() + message.text.len();
                        self.history.push_back(message);
                        while self.history.len() > 100 || self.history_bytes > HISTORY_BYTES {
                            let removed = self.history.pop_front().unwrap();
                            self.history_bytes -= removed.title.len() + removed.text.len();
                        }
                        return None;
                    }
                    self.messages = None;
                }
            }
        }
    }

    pub(crate) fn render(&mut self, buffer: &mut Buffer, palette: &Palette) -> Option<(u16, u16)> {
        if !self.visible {
            return None;
        }
        let area = buffer.area;
        let width = area.width.saturating_sub(4).min(90);
        let height = area
            .height
            .saturating_sub(2)
            .min(if self.history_enabled { 30 } else { 18 });
        let popup = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        Clear.render(popup, buffer);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(self.title.as_str())
            .style(Style::default().fg(palette.text).bg(palette.panel_bg));
        let inner = block.inner(popup);
        block.render(popup, buffer);
        if inner.width == 0 || inner.height < 3 {
            return None;
        }
        let history_height = if self.history_enabled && inner.height >= 7 {
            (inner.height - 4) * 2 / 3
        } else {
            0
        };
        if history_height > 0 {
            let mut lines = Vec::new();
            for message in &self.history {
                lines.extend(wrap(&message.title, inner.width));
                lines.extend(wrap(&message.text, inner.width));
                lines.push(String::new());
            }
            if lines.is_empty() {
                lines.push("No host messages".into());
            }
            self.history_page = usize::from(history_height);
            self.history_scroll = self
                .history_scroll
                .min(lines.len().saturating_sub(self.history_page));
            let top = lines
                .len()
                .saturating_sub(self.history_page + self.history_scroll);
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(top)
                    .take(self.history_page)
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .render(
                Rect::new(inner.x, inner.y, inner.width, history_height),
                buffer,
            );
            Paragraph::new("Message | PgUp/PgDn history")
                .style(Style::default().fg(palette.overlay0))
                .render(
                    Rect::new(inner.x, inner.y + history_height, inner.width, 1),
                    buffer,
                );
        }
        let edit_top = inner.y + history_height + u16::from(history_height > 0);
        let edit = Rect::new(
            inner.x,
            edit_top,
            inner.width,
            inner.bottom() - 2 - edit_top,
        );
        let (lines, (x, y)) = self.editor.multiline_viewport(edit.width, edit.height);
        Paragraph::new(lines.join("\n")).render(edit, buffer);
        Paragraph::new(self.status.as_str()).render(
            Rect::new(inner.x, inner.bottom() - 2, inner.width, 1),
            buffer,
        );
        Paragraph::new("Ctrl+S send | Enter newline | Esc native terminal")
            .style(Style::default().fg(palette.overlay0))
            .render(
                Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
                buffer,
            );
        self.pending.is_none().then_some((edit.x + x, edit.y + y))
    }
}

fn truncate(text: &mut String, limit: usize) {
    if text.len() > limit {
        let mut end = limit.saturating_sub(24);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[display truncated]");
    }
}

fn wrap(text: &str, width: u16) -> Vec<String> {
    let mut lines = Vec::new();
    for line in text.split('\n') {
        let expanded = line.replace('\t', "    ");
        let mut row = String::new();
        let mut cells = 0;
        for grapheme in expanded.graphemes(true) {
            let used = grapheme.width();
            if cells + used > usize::from(width) && !row.is_empty() {
                lines.push(std::mem::take(&mut row));
                cells = 0;
            }
            if !grapheme.chars().any(char::is_control) {
                row.push_str(grapheme);
                cells += used;
            }
        }
        lines.push(row);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;
    use std::sync::Arc;

    #[tokio::test]
    async fn receives_messages_without_cancelling_an_inflight_submission() {
        let accepted = Arc::new(tokio::sync::Notify::new());
        let completion = accepted.clone();
        let (sender, messages) = mpsc::channel(1);
        let mut composer = Composer::new("Host", move |_, _| {
            let completion = completion.clone();
            async move {
                completion.notified().await;
                Ok("Recorded".into())
            }
        })
        .with_messages(messages);
        composer.open();
        composer.handle(&Event::Paste("draft".into()));
        composer.handle(&Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        )));
        sender
            .send(ComposerMessage {
                title: "Reply".into(),
                text: "First line\nSecond line".into(),
            })
            .await
            .unwrap();
        assert_eq!(composer.next().await, None);
        assert!(composer.pending.is_some());
        assert_eq!(composer.editor.as_str(), "draft");
        drop(sender);
        accepted.notify_one();
        assert_eq!(composer.next().await, Some(Ok("Recorded".into())));
        assert!(composer.editor.is_empty());
        assert_eq!(composer.history[0].text, "First line\nSecond line");
    }

    #[tokio::test]
    async fn renders_wrapped_history_and_pages_to_older_messages() {
        let (sender, messages) = mpsc::channel(1);
        let mut composer =
            Composer::new("Host", |_, _| async { Ok(String::new()) }).with_messages(messages);
        for index in 0..12 {
            sender
                .send(ComposerMessage {
                    title: format!("Reply {index}"),
                    text: format!("one \u{754c}\u{754c}\n{}wrapped-tail\ntwo", "A".repeat(24)),
                })
                .await
                .unwrap();
            composer.next().await;
        }
        composer.open();
        let area = Rect::new(0, 0, 30, 20);
        let mut buffer = Buffer::empty(area);
        composer.render(&mut buffer, &Palette::catppuccin());
        let content = |buffer: &Buffer| {
            buffer
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
        };
        assert!(content(&buffer).contains("Reply 11"));
        assert!(content(&buffer).contains("\u{754c}"));
        assert!(content(&buffer).contains("wrapped-tail"));
        for _ in 0..20 {
            composer.handle(&Event::Key(KeyEvent::new(
                KeyCode::PageUp,
                KeyModifiers::NONE,
            )));
        }
        composer.render(&mut buffer, &Palette::catppuccin());
        assert!(content(&buffer).contains("Reply 0"));
        assert!(!content(&buffer).contains("Reply 11"));
    }
}
