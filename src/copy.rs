use crate::vt::Frame;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::io::{self, Write};
/// Frozen viewport selection follows Herdr's anchor/drag/retain interaction.
pub struct Copy {
    pub frame: Frame,
    pub anchor: (u16, u16),
    pub cursor: (u16, u16),
    pub selecting: bool,
    pub query: String,
}
impl Copy {
    pub fn new(frame: Frame) -> Self {
        Self {
            frame,
            anchor: (0, 0),
            cursor: (0, 0),
            selecting: false,
            query: String::new(),
        }
    }
    pub fn contains(&self, x: u16, y: u16) -> bool {
        if !self.selecting {
            return false;
        }
        let a = (self.anchor.1, self.anchor.0);
        let b = (self.cursor.1, self.cursor.0);
        let p = (y, x);
        p >= a.min(b) && p <= a.max(b)
    }
    pub fn text(&self) -> String {
        let mut output = vec![];
        for (y, row) in self.frame.rows.iter().enumerate() {
            let mut line = String::new();
            let mut selected = false;
            let mut tail = false;
            for (x, cell) in row.iter().enumerate() {
                if tail {
                    tail = false;
                    continue;
                }
                tail = cell.wide;
                if self.contains(x as u16, y as u16) {
                    line.push_str(if cell.text.is_empty() {
                        " "
                    } else {
                        &cell.text
                    });
                    selected = true;
                }
            }
            if selected {
                output.push(line.trim_end().to_owned());
            }
        }
        output.join("\n")
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
    let encoded = STANDARD.encode(text);
    write!(io::stdout(), "\x1b]52;c;{encoded}\x07")?;
    io::stdout().flush()
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
        let mut copy = Copy::new(vt.frame());
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
        copy.anchor = (0, 0);
        copy.cursor = (8, 0);
        assert!(copy.text().starts_with("界 needle"));
    }
    #[test]
    fn frozen_selection_and_search() {
        let mut vt = Vt::new(20, 3);
        vt.write(b"hello\r\nworld");
        let mut c = Copy::new(vt.frame());
        c.selecting = true;
        c.cursor = (4, 0);
        assert_eq!(c.text(), "hello");
        assert!(c.search("world"));
        assert_eq!(c.cursor, (0, 1));
        let mut spaced = Vt::new(20, 3);
        spaced.write(b"a\x1b[3Cb");
        let mut search = Copy::new(spaced.frame());
        assert!(search.search("a   b"));
        assert!(!search.search("ab"));
        vt.write(b"changed");
        assert_eq!(c.frame.rows[0][0].text, "h");
    }
}
