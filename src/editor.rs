//! Grapheme-aware editor adapted from Herdr client shell.
use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextEditor {
    text: String,
    cursor: usize,
    replace_on_type: bool,
    killed: String,
    multiline: bool,
}

impl std::ops::Deref for TextEditor {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for TextEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.text.fmt(f)
    }
}

impl From<&str> for TextEditor {
    fn from(text: &str) -> Self {
        Self::new(text, false)
    }
}

impl TextEditor {
    pub fn multiline() -> Self {
        Self {
            multiline: true,
            ..Self::default()
        }
    }

    pub fn new(text: &str, replace_on_type: bool) -> Self {
        let mut editor = Self::default();
        editor.insert(text);
        editor.replace_on_type = replace_on_type;
        editor
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.replace_on_type = false;
    }

    pub fn trim_and_accept(&mut self) {
        let leading = self.text.len() - self.text.trim_start().len();
        self.text = self.text.trim().to_owned();
        self.cursor = self.cursor.saturating_sub(leading).min(self.text.len());
        self.replace_on_type = false;
        self.repair_cursor();
    }

    fn repair_cursor(&mut self) {
        // Insertion/deletion can join clusters across the edit. Snap forward, never
        // leave an insertion offset inside the newly formed grapheme.
        self.cursor = self
            .text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|index| *index >= self.cursor)
            .unwrap_or(self.text.len());
    }

    pub fn insert(&mut self, text: &str) -> bool {
        let mut normalized = String::new();
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    normalized.push(if self.multiline { '\n' } else { ' ' });
                }
                '\n' | '\t' if self.multiline => normalized.push(ch),
                '\n' | '\t' => normalized.push(' '),
                ch if !ch.is_control() => normalized.push(ch),
                _ => {}
            }
        }
        if normalized.is_empty() {
            return false;
        }
        let content_changed = !self.replace_on_type || self.text != normalized;
        if self.replace_on_type {
            self.clear();
        }
        self.text.insert_str(self.cursor, &normalized);
        self.cursor += normalized.len();
        self.repair_cursor();
        content_changed
    }

    fn previous(&self) -> usize {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    fn next(&self) -> usize {
        self.cursor
            + self.text[self.cursor..]
                .graphemes(true)
                .next()
                .map(str::len)
                .unwrap_or(0)
    }

    fn word_boundary(&self, backward: bool) -> usize {
        let class = |grapheme: &str| {
            let ch = grapheme.chars().next().unwrap_or(' ');
            if ch.is_whitespace() {
                0
            } else if ch.is_alphanumeric() || ch == '_' {
                1
            } else {
                2
            }
        };
        let mut boundary = self.cursor;
        let mut run = 0;
        if backward {
            for (index, grapheme) in self.text[..self.cursor].grapheme_indices(true).rev() {
                let current = class(grapheme);
                if run != 0 && current != run {
                    break;
                }
                run = current;
                boundary = index;
            }
        } else {
            for (index, grapheme) in self.text[self.cursor..].grapheme_indices(true) {
                let current = class(grapheme);
                if run != 0 && current != run {
                    break;
                }
                run = current;
                boundary = self.cursor + index + grapheme.len();
            }
        }
        boundary
    }

    fn remove(&mut self, start: usize, end: usize, kill: bool) {
        self.replace_on_type = false;
        if start == end {
            return;
        }
        if kill {
            self.killed = self.text[start..end].to_owned();
        }
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.repair_cursor();
    }

    pub fn handle_key(&mut self, key: &crossterm::event::KeyEvent) -> Option<bool> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if key.code == KeyCode::Enter && self.multiline {
            return Some(self.insert("\n"));
        }
        if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
            return None;
        }
        if self.multiline && key.modifiers.is_empty() {
            let start = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
            let end = self.text[self.cursor..]
                .find('\n')
                .map_or(self.text.len(), |i| self.cursor + i);
            let target = match key.code {
                KeyCode::Home => Some(start),
                KeyCode::End => Some(end),
                KeyCode::Up | KeyCode::Down => {
                    let column = self.text[start..self.cursor].graphemes(true).count();
                    let line = if key.code == KeyCode::Up && start > 0 {
                        let previous = self.text[..start - 1].rfind('\n').map_or(0, |i| i + 1);
                        Some((previous, start - 1))
                    } else if key.code == KeyCode::Down && end < self.text.len() {
                        let next_end = self.text[end + 1..]
                            .find('\n')
                            .map_or(self.text.len(), |i| end + 1 + i);
                        Some((end + 1, next_end))
                    } else {
                        None
                    };
                    line.map(|(a, b)| {
                        a + self.text[a..b]
                            .graphemes(true)
                            .take(column)
                            .map(str::len)
                            .sum::<usize>()
                    })
                }
                _ => None,
            };
            if let Some(target) = target {
                self.cursor = target;
                self.replace_on_type = false;
                return Some(false);
            }
        }
        let previous_len = self.text.len();
        let mut content_changed = false;
        let (code, modifiers) = (key.code, key.modifiers);
        {
            let ctrl = modifiers == KeyModifiers::CONTROL;
            let alt = modifiers == KeyModifiers::ALT;
            let plain = modifiers.is_empty();
            let movement = match code {
                KeyCode::Left if plain => Some(self.previous()),
                KeyCode::Char('b') if ctrl => Some(self.previous()),
                KeyCode::Right if plain => Some(self.next()),
                KeyCode::Char('f') if ctrl => Some(self.next()),
                KeyCode::Home if plain => Some(0),
                KeyCode::Char('a') if ctrl => Some(0),
                KeyCode::End if plain => Some(self.text.len()),
                KeyCode::Char('e') if ctrl => Some(self.text.len()),
                KeyCode::Char('b') if alt => Some(self.word_boundary(true)),
                KeyCode::Char('f') if alt => Some(self.word_boundary(false)),
                _ => None,
            };
            if let Some(position) = movement {
                self.cursor = position;
                self.replace_on_type = false;
            } else {
                match code {
                    KeyCode::Backspace | KeyCode::Char('h')
                        if code == KeyCode::Backspace && plain
                            || code == KeyCode::Char('h') && ctrl =>
                    {
                        if self.replace_on_type {
                            self.clear();
                        } else {
                            self.remove(self.previous(), self.cursor, false);
                        }
                    }
                    KeyCode::Delete if plain => self.remove(self.cursor, self.next(), false),
                    KeyCode::Char('d') if ctrl => self.remove(self.cursor, self.next(), false),
                    KeyCode::Char('u') if ctrl => self.remove(0, self.cursor, true),
                    KeyCode::Char('k') if ctrl => self.remove(self.cursor, self.text.len(), true),
                    KeyCode::Char('w') if ctrl => {
                        self.remove(self.word_boundary(true), self.cursor, true)
                    }
                    KeyCode::Backspace if ctrl || alt => {
                        self.remove(self.word_boundary(true), self.cursor, true)
                    }
                    KeyCode::Char('d') if alt => {
                        self.remove(self.cursor, self.word_boundary(false), true)
                    }
                    KeyCode::Char('y') if ctrl => {
                        content_changed = self.insert(&self.killed.clone());
                    }
                    KeyCode::Char(_) if modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
                        if let KeyCode::Char(ch) = key.code {
                            content_changed = self.insert(&ch.to_string());
                        }
                    }
                    _ => return None,
                }
            }
        }
        Some(content_changed || self.text.len() != previous_len)
    }

    pub fn viewport(&self, width: u16) -> (&str, u16) {
        if width == 0 {
            return ("", 0);
        }
        let mut start = self.cursor;
        let mut cells = 0;
        for (index, grapheme) in self.text[..self.cursor].grapheme_indices(true).rev() {
            let next = cells + grapheme.width();
            if next >= usize::from(width) {
                break;
            }
            start = index;
            cells = next;
        }
        let mut end = self.cursor;
        let mut used = cells;
        for (index, grapheme) in self.text[self.cursor..].grapheme_indices(true) {
            used += grapheme.width();
            if used > usize::from(width) {
                break;
            }
            end = self.cursor + index + grapheme.len();
        }
        (&self.text[start..end], cells as u16)
    }

    pub fn multiline_viewport(&self, width: u16, height: u16) -> (Vec<String>, (u16, u16)) {
        let prefix = &self.text[..self.cursor];
        let row = prefix.bytes().filter(|b| *b == b'\n').count();
        let column = prefix
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .replace('\t', "    ")
            .width();
        let left = column.saturating_sub(usize::from(width.saturating_sub(1)));
        let top = row.saturating_sub(usize::from(height.saturating_sub(1)));
        let lines = self
            .text
            .split('\n')
            .skip(top)
            .take(usize::from(height))
            .map(|line| {
                let expanded = line.replace('\t', "    ");
                let mut cells = 0;
                let mut visible = String::new();
                for grapheme in expanded.graphemes(true) {
                    let end = cells + grapheme.width();
                    if cells >= left && end <= left + usize::from(width) {
                        visible.push_str(grapheme);
                    } else if cells < left && end > left {
                        visible.push_str(&" ".repeat(end - left));
                    }
                    cells = end;
                }
                visible
            })
            .collect();
        (lines, ((column - left) as u16, (row - top) as u16))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_graphemes_without_splitting_utf8() {
        let mut editor = TextEditor::new("界e\u{301}", false);
        editor.handle_key(&crossterm::event::KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        ));
        assert_eq!(editor.as_str(), "界");
        editor.handle_key(&crossterm::event::KeyEvent::new(
            KeyCode::Home,
            KeyModifiers::NONE,
        ));
        editor.insert("a");
        assert_eq!(editor.as_str(), "a界");
        assert_eq!(editor.viewport(2), ("a", 1));
    }

    #[test]
    fn multiline_editing_retains_whitespace_and_moves_between_lines() {
        use crossterm::event::KeyEvent;
        let mut editor = TextEditor::multiline();
        let raw = "  hello\n\t\u{754c}e\u{301}  \n";
        editor.insert(raw);
        assert_eq!(editor.as_str(), raw);
        editor.handle_key(&KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        editor.handle_key(&KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        editor.handle_key(&KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(editor.as_str(), "  hello\n\t\u{754c}e\u{301} \n");
        editor.handle_key(&KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        editor.handle_key(&KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(editor.as_str(), "  hello\n\t\u{754c}\n");
        let (lines, cursor) = editor.multiline_viewport(5, 2);
        assert_eq!(cursor, (4, 1));
        assert_eq!(lines[1], "  \u{754c}");
    }
}
