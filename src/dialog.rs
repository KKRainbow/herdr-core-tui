use crate::{Result, editor::TextEditor};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde_json::{Value, json};
pub struct Field {
    pub name: String,
    pub editor: TextEditor,
}
pub struct Form {
    pub defaults: Value,
    pub command: String,
    pub fields: Vec<Field>,
    pub active: usize,
}
pub enum Dialog {
    Menu {
        selected: usize,
    },
    Form(Form),
    Message(String),
    Confirm {
        command: String,
        args: Value,
        description: String,
    },
}
impl Form {
    pub fn new(command: &str, defaults: &Value) -> Self {
        let names: &[&str] = match command {
            "space.new" => &["name", "folder", "line"],
            "tab.new" => &["name", "line"],
            "pane.split" => &["axis", "line"],
            "space.rename" | "tab.rename" | "pane.rename" => &["name"],
            "space.move" | "tab.move" => &["index"],
            "pane.move" => &["destination", "index"],
            "pane.swap" => &["other"],
            "pane.resize" => &["weight"],
            "terminal.keep" => &["terminal"],
            "worktree.new" => &["repository", "branch", "path", "new_branch"],
            "worktree.open" => &["path", "name"],
            "worktree.list" | "worktree.branches" => &["repository"],
            "worktree.remove" => &["repository", "path"],
            "machine" => &["machine"],
            "search" => &["query"],
            "command" => &["command", "arguments"],
            _ => &[],
        };
        Self {
            defaults: defaults.clone(),
            command: command.into(),
            active: 0,
            fields: names
                .iter()
                .map(|name| {
                    let value = defaults[*name]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            if defaults[*name].is_null() {
                                String::new()
                            } else {
                                defaults[*name].to_string()
                            }
                        });
                    Field {
                        name: (*name).into(),
                        editor: TextEditor::new(&value, true),
                    }
                })
                .collect(),
        }
    }
    pub fn handle(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                self.active = (self.active + 1) % self.fields.len().max(1)
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.active =
                    (self.active + self.fields.len().saturating_sub(1)) % self.fields.len().max(1)
            }
            _ => {
                if let Some(field) = self.fields.get_mut(self.active) {
                    field.editor.handle_key(&key);
                }
            }
        }
    }
    pub fn values(&mut self) -> Result<Value> {
        let mut result = self.defaults.clone();
        for field in &mut self.fields {
            field.editor.trim_and_accept();
            let s = field.editor.as_str();
            if s.is_empty() {
                continue;
            }
            result[&field.name] = if matches!(
                field.name.as_str(),
                "argv" | "index" | "weight" | "new_branch" | "arguments"
            ) {
                serde_json::from_str(s)?
            } else {
                json!(s)
            };
        }
        Ok(result)
    }
}
pub fn render(frame: &mut Frame, dialog: &Dialog) {
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(90);
    let height = screen.height.saturating_sub(2).min(match dialog {
        Dialog::Form(f) => (f.fields.len() * 3 + 4) as u16,
        _ => 22,
    });
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    let title = match dialog {
        Dialog::Menu { .. } => " Actions · arrows / Enter · Esc closes ".into(),
        Dialog::Form(f) => format!(" {} · Tab moves · Enter submits · Esc cancels ", f.command),
        Dialog::Message(_) => " Esc closes ".into(),
        Dialog::Confirm { .. } => " Confirm · y accepts · Esc cancels ".into(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match dialog {
        Dialog::Menu { selected } => {
            let start = selected.saturating_sub(inner.height.saturating_sub(1) as usize);
            for (index, (label, _)) in MENU
                .iter()
                .enumerate()
                .skip(start)
                .take(inner.height as usize)
            {
                frame.render_widget(
                    Paragraph::new(*label).style(Style::default().fg(if index == *selected {
                        Color::Yellow
                    } else {
                        Color::White
                    })),
                    Rect::new(inner.x, inner.y + (index - start) as u16, inner.width, 1),
                );
            }
        }
        Dialog::Message(text) => frame.render_widget(
            Paragraph::new(text.as_str()).wrap(Wrap { trim: false }),
            inner,
        ),
        Dialog::Confirm { description, .. } => frame.render_widget(
            Paragraph::new(description.as_str()).wrap(Wrap { trim: false }),
            inner,
        ),
        Dialog::Form(form) => {
            for (i, field) in form.fields.iter().enumerate() {
                let y = inner.y + i as u16 * 3;
                if y + 1 >= inner.bottom() {
                    break;
                }
                frame.render_widget(
                    Paragraph::new(field.name.as_str()),
                    Rect::new(inner.x, y, inner.width, 1),
                );
                let (text, cursor) = field.editor.viewport(inner.width);
                frame.render_widget(
                    Paragraph::new(text).style(Style::default().fg(if i == form.active {
                        Color::Yellow
                    } else {
                        Color::White
                    })),
                    Rect::new(inner.x, y + 1, inner.width, 1),
                );
                if i == form.active {
                    frame.set_cursor_position((inner.x + cursor, y + 1));
                }
            }
        }
    }
}

pub const MENU: &[(&str, &str)] = &[
    ("New space", "space.new"),
    ("New tab", "tab.new"),
    ("Split right", "split.row"),
    ("Split down", "split.column"),
    ("Next pane", "pane.next"),
    ("Next tab", "tab.next"),
    ("Zoom pane", "zoom"),
    ("Rename pane", "pane.rename"),
    ("Rename tab", "tab.rename"),
    ("Rename space", "space.rename"),
    ("Move pane", "pane.move"),
    ("Swap panes", "pane.swap"),
    ("Resize pane", "pane.resize"),
    ("Reorder tab", "tab.move"),
    ("Reorder space", "space.move"),
    ("Close pane", "pane.close"),
    ("Close tab", "tab.close"),
    ("Close space", "space.close"),
    ("Keep terminal", "terminal.keep"),
    ("Park terminal", "terminal.park"),
    ("Copy mode", "copy"),
    ("List worktrees", "worktree.list"),
    ("List branches", "worktree.branches"),
    ("Create worktree", "worktree.new"),
    ("Open worktree", "worktree.open"),
    ("Remove worktree", "worktree.remove"),
    ("Select machine", "machine"),
    ("Command", "command"),
    ("Help", "help"),
];
