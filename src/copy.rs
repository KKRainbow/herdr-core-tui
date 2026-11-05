use crate::vt::{Frame, TextMatch, Vt};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
    pub search_backward: bool,
    pub matches: Vec<TextMatch>,
    pub current_match: Option<usize>,
    match_text: String,
    search_revision: u64,
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
            search_backward: false,
            matches: Vec::new(),
            current_match: None,
            match_text: String::new(),
            search_revision: 0,
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
    pub fn rebase(&mut self, vt: &mut Vt) {
        // A deliberate viewport refresh must show the same live content we search.
        vt.scroll_row(vt.top_row());
        if !self.query.is_empty() && self.search_revision != vt.revision() {
            let current = self
                .current_match
                .and_then(|i| self.matches.get(i))
                .copied();
            self.matches = vt.search(&self.query);
            self.current_match =
                current.and_then(|found| self.matches.iter().position(|m| *m == found));
            self.capture_match(vt);
        }
        self.frame = vt.frame();
        self.top = vt.top_row();
        self.cursor.0 = self.cursor.0.min(vt.cols - 1);
        self.cursor.1 = self.cursor.1.min(vt.rows - 1);
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
    /// The explicit selection, or the current match from the frozen search frame.
    pub fn text(&self, vt: &crate::vt::Vt) -> String {
        if self.selecting {
            vt.selection_text(self.anchor, self.cursor_point())
        } else {
            self.match_text.clone()
        }
    }
    pub fn clear_search(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.current_match = None;
        self.match_text.clear();
    }
    pub fn search(&mut self, vt: &mut Vt, query: &str, backward: bool) -> bool {
        if query.is_empty() {
            return false;
        }
        self.query = query.into();
        self.search_backward = backward;
        self.find(vt, backward, false)
    }
    pub fn repeat_search(&mut self, vt: &mut Vt, reverse: bool) -> bool {
        if self.query.is_empty() {
            return false;
        }
        let skip_current = self
            .current_match
            .and_then(|i| self.matches.get(i))
            .is_some_and(|found| found.start == self.cursor_point());
        self.find(vt, self.search_backward ^ reverse, skip_current)
    }
    fn find(&mut self, vt: &mut Vt, backward: bool, skip_current: bool) -> bool {
        let (x, y) = self.cursor_point();
        self.matches = vt.search(&self.query);
        self.current_match = None;
        if self.matches.is_empty() {
            self.capture_match(vt);
            return false;
        }
        let eligible = |found: &TextMatch| {
            let point = (found.start.1, found.start.0);
            if backward {
                point < (y, x) || (!skip_current && point == (y, x))
            } else {
                point > (y, x) || (!skip_current && point == (y, x))
            }
        };
        let index = if backward {
            self.matches
                .iter()
                .rposition(eligible)
                .unwrap_or(self.matches.len() - 1)
        } else {
            self.matches.iter().position(eligible).unwrap_or(0)
        };
        self.current_match = Some(index);
        let found = self.matches[index];
        // Keep the match above the mode bar when revealing another history page.
        let last_visible = u64::from(vt.rows.saturating_sub(2));
        let top = vt.top_row();
        let top = if found.start.1 < top {
            found.start.1
        } else if found.start.1 > top + last_visible {
            found.start.1.saturating_sub(last_visible)
        } else {
            top
        };
        // Searching reads live content, including writes inside synchronized output.
        vt.scroll_row(top);
        self.capture_match(vt);
        self.rebase(vt);
        self.cursor = (found.start.0, (found.start.1 - self.top) as u16);
        true
    }
    fn capture_match(&mut self, vt: &Vt) {
        self.match_text = self
            .current_match
            .and_then(|i| self.matches.get(i))
            .map(|found| vt.selection_text(found.start, found.end))
            .unwrap_or_default();
        self.search_revision = vt.revision();
    }
    /// Whether a cell belongs to a search match, and whether that match is current.
    pub fn search_contains(&self, x: u16, y: u16) -> Option<bool> {
        let point = (self.top + u64::from(y), x);
        let index = self
            .matches
            .partition_point(|found| (found.end.1, found.end.0) < point);
        let found = self.matches.get(index)?;
        ((found.start.1, found.start.0) <= point).then_some(self.current_match == Some(index))
    }
    pub fn page(&mut self, vt: &mut Vt, backward: bool, half: bool) {
        let height = u64::from(vt.rows);
        let lines = if height <= 2 {
            1
        } else if half {
            height / 2
        } else {
            height - 2
        };
        let (_, total) = vt.scroll_position();
        let shift = |row: u64| {
            if backward {
                row.saturating_sub(lines)
            } else {
                row.saturating_add(lines)
            }
        };
        let row = shift(self.cursor_point().1).min(total.saturating_sub(1));
        vt.scroll_row(shift(self.top).min(total.saturating_sub(height)));
        self.rebase(vt);
        self.cursor.1 = row.saturating_sub(self.top).min(height - 1) as u16;
    }
    pub fn history(&mut self, vt: &mut Vt, top: bool) {
        let (_, total) = vt.scroll_position();
        vt.scroll_row(if top { 0 } else { total });
        self.rebase(vt);
        self.cursor.1 = if top {
            0
        } else {
            total.saturating_sub(self.top + 1) as u16
        };
    }
}
pub fn page_key(key: KeyEvent) -> Option<(bool, bool)> {
    match key.code {
        KeyCode::PageUp => Some((true, false)),
        KeyCode::PageDown => Some((false, false)),
        KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) => match ch {
            'b' => Some((true, false)),
            'f' => Some((false, false)),
            'u' => Some((true, true)),
            'd' => Some((false, true)),
            _ => None,
        },
        _ => None,
    }
}
pub fn clipboard(text: &str) -> io::Result<()> {
    crate::clipboard::copy(text)
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
        let mut copy = Copy::new(vt.frame(), vt.top_row());
        assert!(copy.search(&mut vt, "needle", false));
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
        assert!(c.search(&mut vt, "world", false));
        assert_eq!(c.cursor, (0, 1));
        let mut spaced = Vt::new(20, 3);
        spaced.write(b"a\x1b[3Cb");
        let mut search = Copy::new(spaced.frame(), 0);
        assert!(search.search(&mut spaced, "a   b", false));
        assert!(!search.search(&mut spaced, "ab", false));
        vt.write(b"changed");
        assert_eq!(c.frame.rows[0][0].text, "h");
    }

    #[test]
    fn search_visits_each_match_and_wraps_in_both_directions() {
        let mut vt = Vt::new(30, 5);
        vt.write(b"hit hit\r\nother\r\nhit\r\nend");
        let mut copy = Copy::new(vt.frame(), vt.top_row());
        assert!(copy.search(&mut vt, "hit", false));
        assert_eq!(copy.matches.len(), 3);
        assert_eq!(copy.current_match, Some(0));
        assert_eq!(copy.cursor_point(), (0, 0));
        for (index, point) in [(1, (4, 0)), (2, (0, 2)), (0, (0, 0))] {
            assert!(copy.repeat_search(&mut vt, false));
            assert_eq!(copy.current_match, Some(index));
            assert_eq!(copy.cursor_point(), point);
        }
        assert!(copy.repeat_search(&mut vt, true));
        assert_eq!(copy.current_match, Some(2));

        copy.cursor = (0, 1);
        assert!(copy.search(&mut vt, "hit", true));
        assert_eq!(copy.current_match, Some(1));
        assert!(copy.repeat_search(&mut vt, false));
        assert_eq!(copy.current_match, Some(0));
        assert!(copy.repeat_search(&mut vt, true));
        assert_eq!(copy.current_match, Some(1));
        assert!(copy.repeat_search(&mut vt, false));
        assert_eq!(copy.current_match, Some(0));

        // Moving away from a match makes the cursor the next search's origin.
        copy.cursor = (0, 1);
        assert!(copy.repeat_search(&mut vt, false));
        assert_eq!(copy.current_match, Some(1));
    }

    #[test]
    fn search_uses_smart_case_and_preserves_spaces() {
        let mut vt = Vt::new(40, 4);
        vt.write(b"Error error ERROR\r\n Foo \r\nend");
        let mut copy = Copy::new(vt.frame(), vt.top_row());
        assert!(copy.search(&mut vt, "error", false));
        assert_eq!(copy.matches.len(), 3);
        assert_eq!(copy.text(&vt), "Error");
        assert!(copy.search(&mut vt, "Error", false));
        assert_eq!(copy.matches.len(), 1);
        assert_eq!(copy.text(&vt), "Error");
        // Ghostty's search text excludes padding at the end of a hard line.
        assert!(!copy.search(&mut vt, " Foo ", false));
        vt.write(b"\r Foo rest");
        assert!(copy.search(&mut vt, " Foo ", false));
        assert_eq!(copy.matches.len(), 1);
        assert_eq!(copy.matches[0].start, (0, 2));
        assert_eq!(copy.matches[0].end, (4, 2));
    }

    #[test]
    fn search_handles_wide_combining_and_soft_wrapped_text() {
        let mut vt = Vt::new(10, 6);
        vt.write("界hit 界hit\r\ne\u{301}needle\r\nend".as_bytes());
        let mut copy = Copy::new(vt.frame(), vt.top_row());
        assert!(copy.search(&mut vt, "hit", false));
        assert_eq!(copy.matches.len(), 2);
        assert_eq!(copy.matches[0].start, (2, 0));
        assert_eq!(copy.matches[1].start, (8, 0));
        assert_eq!(copy.matches[1].end, (0, 1));
        assert!(copy.repeat_search(&mut vt, false));
        assert_eq!(copy.text(&vt), "hit");
        assert_eq!(copy.search_contains(8, 0), Some(true));
        assert_eq!(copy.search_contains(0, 1), Some(true));
        assert_eq!(copy.search_contains(1, 1), None);
        assert!(copy.search(&mut vt, "needle", false));
        assert_eq!(copy.cursor_point(), (1, 2));
        assert_eq!(copy.text(&vt), "needle");
    }

    #[test]
    fn missing_and_empty_searches_leave_viewport_and_selection_in_place() {
        let mut vt = Vt::new(20, 3);
        vt.write(b"hit\r\n1\r\n2\r\n3\r\n4\r\n5");
        let mut copy = Copy::new(vt.frame(), vt.top_row());
        copy.cursor = (0, 1);
        copy.anchor_here();
        copy.selecting = true;
        let before = (vt.scroll_position(), copy.cursor_point(), copy.anchor);
        assert!(!copy.search(&mut vt, "missing", true));
        assert_eq!(copy.current_match, None);
        assert!(copy.matches.is_empty());
        assert!(copy.selecting);
        assert_eq!(
            (vt.scroll_position(), copy.cursor_point(), copy.anchor),
            before
        );
        assert!(!copy.search(&mut vt, "", false));
        assert_eq!(copy.query, "missing");
        assert!(copy.search_backward);
        assert!(copy.search(&mut vt, "hit", true));
        assert_eq!(copy.anchor, before.2);
        assert!(copy.selecting);
        assert!(copy.text(&vt).starts_with("hit"));
        copy.clear_search();
        assert!(copy.matches.is_empty());
        assert_eq!(copy.current_match, None);
        assert_eq!(copy.search_contains(0, 0), None);
    }

    #[test]
    fn copying_a_match_uses_the_text_of_the_frozen_frame() {
        let mut vt = Vt::new(20, 3);
        vt.write(b"hit");
        let mut copy = Copy::new(vt.frame(), vt.top_row());
        assert!(copy.search(&mut vt, "hit", false));
        vt.write(b"\rbad");
        assert_eq!(copy.frame.rows[0][0].text, "h");
        assert_eq!(copy.text(&vt), "hit");

        // A deliberate viewport refresh updates both the frame and its matches.
        copy.page(&mut vt, true, false);
        assert_eq!(copy.frame.rows[0][0].text, "b");
        assert!(copy.matches.is_empty());
        assert_eq!(copy.current_match, None);
        assert!(copy.text(&vt).is_empty());
    }

    #[test]
    fn refreshing_matches_preserves_the_current_range_when_its_index_changes() {
        let mut vt = Vt::new(20, 4);
        vt.write(b"hit\r\nHIT");
        let mut copy = Copy::new(vt.frame(), vt.top_row());
        copy.cursor = (0, 1);
        assert!(copy.search(&mut vt, "hit", false));
        assert_eq!(copy.current_match, Some(1));
        vt.write(b"\x1b[1;1Hbad");
        copy.page(&mut vt, true, false);
        assert_eq!(copy.matches.len(), 1);
        assert_eq!(copy.current_match, Some(0));
        assert_eq!(copy.text(&vt), "HIT");
        assert_eq!(copy.search_contains(0, 0), None);
        assert_eq!(copy.search_contains(0, 1), Some(true));

        vt.resize(2, 3);
        assert_eq!(copy.text(&vt), "HIT");
        copy.history(&mut vt, true);
        assert_eq!(copy.search_contains(0, 0), None);
        assert_eq!(copy.matches.len(), 1);
    }

    #[test]
    fn searching_synchronized_output_freezes_the_live_match() {
        let mut vt = Vt::new(20, 3);
        vt.write(b"old");
        vt.frame();
        vt.write(b"\x1b[?2026h\rnew");
        let mut copy = Copy::new(vt.frame(), vt.top_row());
        assert_eq!(copy.frame.rows[0][0].text, "o");
        assert!(copy.search(&mut vt, "new", false));
        assert_eq!(copy.frame.rows[0][0].text, "n");
        assert_eq!(copy.text(&vt), "new");
    }

    #[test]
    fn scrolling_during_synchronized_output_refreshes_the_frame_and_matches() {
        let mut vt = Vt::new(20, 3);
        vt.write(b"hit");
        let mut copy = Copy::new(vt.frame(), vt.top_row());
        assert!(copy.search(&mut vt, "hit", false));
        vt.write(b"\x1b[?2026h\rHIT");
        vt.scroll_viewport(-1);
        copy.rebase(&mut vt);
        assert_eq!(copy.frame.rows[0][0].text, "H");
        assert_eq!(copy.text(&vt), "HIT");
        assert_eq!(copy.current_match, Some(0));
    }

    #[test]
    fn page_and_history_motions_move_the_cursor_and_keep_the_anchor() {
        let mut vt = Vt::new(20, 6);
        vt.write(
            (0..30)
                .map(|i| format!("line{i}"))
                .collect::<Vec<_>>()
                .join("\r\n")
                .as_bytes(),
        );
        let mut copy = Copy::new(vt.frame(), vt.top_row());
        copy.history(&mut vt, false);
        copy.anchor_here();
        copy.selecting = true;
        let anchor = copy.anchor;
        assert_eq!(copy.cursor_point(), (0, 29));
        copy.page(&mut vt, true, false);
        assert_eq!((copy.top, copy.cursor_point()), (20, (0, 25)));
        copy.page(&mut vt, true, true);
        assert_eq!((copy.top, copy.cursor_point()), (17, (0, 22)));
        copy.page(&mut vt, false, false);
        assert_eq!((copy.top, copy.cursor_point()), (21, (0, 26)));
        copy.page(&mut vt, false, true);
        assert_eq!((copy.top, copy.cursor_point()), (24, (0, 29)));
        copy.page(&mut vt, false, false);
        assert_eq!((copy.top, copy.cursor_point()), (24, (0, 29)));
        copy.history(&mut vt, true);
        assert_eq!((copy.top, copy.cursor_point()), (0, (0, 0)));
        copy.page(&mut vt, true, false);
        assert_eq!((copy.top, copy.cursor_point()), (0, (0, 0)));
        assert_eq!(copy.anchor, anchor);
        assert_eq!(copy.text(&vt).lines().count(), 30);
    }

    #[test]
    fn paging_works_in_short_viewports() {
        for height in [1, 2] {
            let mut vt = Vt::new(10, height);
            vt.write(b"0\r\n1\r\n2\r\n3");
            let mut copy = Copy::new(vt.frame(), vt.top_row());
            copy.page(&mut vt, true, false);
            assert_eq!(copy.top, 3 - u64::from(height));
            assert!(copy.cursor.1 < height);
            copy.page(&mut vt, false, true);
            assert_eq!(copy.top, 4 - u64::from(height));
            assert!(copy.cursor.1 < height);
        }
    }
}
