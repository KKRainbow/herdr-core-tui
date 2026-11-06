//! Optional host-owned message entry. Core and native terminal input stay separate.
use crate::{editor::TextEditor, shell::palette::Palette};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};
use std::{future::Future, pin::Pin};

type Submission = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;

/// A host supplies submission and receipt text; the TUI only owns editing.
pub struct Composer {
    title: String,
    submit: Box<dyn Fn(u64, String) -> Submission + Send>,
    editor: TextEditor,
    pub(crate) visible: bool,
    revision: u64,
    pending: Option<Submission>,
    status: String,
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
        }
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

    pub(crate) async fn next(&mut self) -> Result<String, String> {
        let result = match &mut self.pending {
            Some(pending) => pending.await,
            None => std::future::pending().await,
        };
        self.pending = None;
        match &result {
            Ok(receipt) => {
                self.editor.clear();
                self.revision += 1;
                self.status = receipt.clone();
            }
            Err(error) => self.status = error.clone(),
        }
        result
    }

    pub(crate) fn render(&self, buffer: &mut Buffer, palette: &Palette) -> Option<(u16, u16)> {
        if !self.visible {
            return None;
        }
        let area = buffer.area;
        let width = area.width.saturating_sub(4).min(90);
        let height = area.height.saturating_sub(2).min(18);
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
        let edit = Rect::new(inner.x, inner.y, inner.width, inner.height - 2);
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
