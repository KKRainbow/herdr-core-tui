use crate::vt::Frame;
use std::io;
/// Selection over a frozen viewport that follows Herdr's anchor/drag/retain interaction.
/// The anchor stays in history while the viewport scrolls, so a selection spans pages.
pub struct Copy {
    pub frame: Frame,
    /// The screen row (history and screen, from the oldest line) of the frame's first row.
    pub top: u64,
    /// Where the selection starts, as (column, screen row).
    pub anchor: (u16, u64),
    /// The cursor in the frame.
    pub cursor: (u16, u16),
    pub selecting: bool,
    pub query: String,
}
impl Copy {
    pub fn new(frame: Frame, top: u64) -> Self {
        Self {
            frame,
            top,
            anchor: (0, top),
            cursor: (0, 0),
            selecting: false,
            query: String::new(),
        }
    }
    /// The cursor as (column, screen row).
    pub fn cursor_point(&self) -> (u16, u64) {
        (self.cursor.0, self.top + u64::from(self.cursor.1))
    }
    /// Starts the selection at the cursor.
    pub fn anchor_here(&mut self) {
        self.anchor = self.cursor_point();
    }
    /// Shows another part of history; the anchor stays where it was.
    pub fn rebase(&mut self, frame: Frame, top: u64) {
        self.frame = frame;
        self.top = top;
    }
    pub fn contains(&self, x: u16, y: u16) -> bool {
        if !self.selecting {
            return false;
        }
        let a = (self.anchor.1, self.anchor.0);
        let cursor = self.cursor_point();
        let b = (cursor.1, cursor.0);
        let p = (self.top + u64::from(y), x);
        p >= a.min(b) && p <= a.max(b)
    }
    /// The selected text across history, read from the terminal.
    pub fn text(&self, vt: &crate::vt::Vt) -> String {
        if !self.selecting {
            return String::new();
        }
        vt.selection_text(self.anchor, self.cursor_point())
    }
    pub fn search(&mut self, query: &str) -> bool {
        self.query = query.into();
        if query.is_empty() {
            return false;
        }
        for offset in 1..=self.frame.rows.len() {
            let y = (self.cursor.1 as usize + offset) % self.frame.rows.len();
            let row = &self.frame.rows[y];
            let mut line = String::new();
            let mut columns = Vec::new();
            let mut tail = false;
            for (x, cell) in row.iter().enumerate() {
                if tail {
                    tail = false;
                    continue;
                }
                tail = cell.wide;
                let text = if cell.text.is_empty() {
                    " "
                } else {
                    &cell.text
                };
                columns.extend(std::iter::repeat_n(x as u16, text.len()));
                line.push_str(text);
            }
            if let Some(index) = line.find(query) {
                self.cursor = (columns[index], y as u16);
                return true;
            }
        }
        false
    }
}
pub fn clipboard(text: &str) -> io::Result<()> {
    crate::clipboard::copy(text)
}

pub fn search_history(vt: &mut crate::vt::Vt, query: &str) -> Option<Copy> {
    if query.is_empty() {
        return None;
    }
    let (original, total) = vt.scroll_position();
    let step = u64::from(vt.rows.max(1));
    for page in 0..=total / step {
        let offset = (original + page * step) % total.max(1);
        vt.scroll_row(offset);
        let mut copy = Copy::new(vt.frame(), vt.top_row());
        if copy.search(query) {
            return Some(copy);
        }
    }
    vt.scroll_row(original);
    None
}
pub fn link(vt: &mut crate::vt::Vt, x: u16, y: u16) -> Option<String> {
    if let Some(link) = vt.hyperlink(x, y) {
        return Some(link);
    }
    let frame = vt.frame();
    let row = frame.rows.get(y as usize)?;
    let mut start = x as usize;
    while start > 0 && !row.get(start - 1)?.text.chars().all(char::is_whitespace) {
        start -= 1;
    }
    let mut end = x as usize;
    while end < row.len() && !row[end].text.chars().all(char::is_whitespace) {
        end += 1;
    }
    let text = row
        .get(start..end)?
        .iter()
        .map(|c| c.text.as_str())
        .collect::<String>();
    (text.starts_with("https://") || text.starts_with("http://")).then_some(text)
}
pub fn open_link(url: &str) -> io::Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(io::Error::other("Only HTTP and HTTPS links can be opened"));
    }
    std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::vt::Vt;
    #[test]
    fn searches_retained_history_and_copies_wide_cells() {
        let mut vt = Vt::new(20, 3);
        vt.write("界 needle\r\n1\r\n2\r\n3\r\n4".as_bytes());
        let mut copy = search_history(&mut vt, "needle").unwrap();
        assert_eq!(copy.cursor, (3, 0));
        copy.selecting = true;
        copy.anchor = (0, copy.top);
        copy.cursor = (8, 0);
        assert!(copy.text(&vt).starts_with("界 needle"));
    }
    #[test]
    fn frozen_selection_and_search() {
        let mut vt = Vt::new(20, 3);
        vt.write(b"hello\r\nworld");
        let mut c = Copy::new(vt.frame(), vt.top_row());
        c.selecting = true;
        c.cursor = (4, 0);
        assert_eq!(c.text(&vt), "hello");
        assert!(c.search("world"));
        assert_eq!(c.cursor, (0, 1));
        let mut spaced = Vt::new(20, 3);
        spaced.write(b"a\x1b[3Cb");
        let mut search = Copy::new(spaced.frame(), 0);
        assert!(search.search("a   b"));
        assert!(!search.search("ab"));
        vt.write(b"changed");
        assert_eq!(c.frame.rows[0][0].text, "h");
    }
}
