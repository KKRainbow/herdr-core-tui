//! Herdr's interface: chrome, overlays, and keymap adapted from Herdr's client shell
//! and fed by `herdr-client-lib` documents and core terminal facts.
pub mod chrome;
pub mod keys;
pub mod overlays;
pub mod palette;
pub mod panes;
pub mod snapshot;

use chrome::ScrollMetrics;
use palette::Palette;
use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Spaces,
    Priority,
}

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub palette: Palette,
    /// Herdr's `status_indicators = "symbols"`; dots otherwise.
    pub symbols: bool,
    pub sort: Sort,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Terminal,
    Prefix,
    Navigate,
    Resize,
    Copy,
}

/// What the copy-mode bar reports.
pub struct CopyBar {
    pub search_prompt: Option<String>,
    pub query: String,
    pub matched: bool,
    pub selecting: bool,
}

#[derive(Clone, Debug)]
pub struct WorkspaceHit {
    pub rect: Rect,
    pub workspace_id: String,
    pub group_toggle: Option<(Rect, String)>,
}

/// Where the chrome drew its clickable parts during the last frame.
#[derive(Default)]
pub struct Hits {
    pub workspaces: Vec<WorkspaceHit>,
    pub agents: Vec<(Rect, String)>,
    pub tabs: Vec<(Rect, String)>,
    pub new_workspace: Rect,
    pub global_launcher: Rect,
    pub sidebar_toggle: Rect,
    pub sidebar_divider: Rect,
    pub sidebar_section_divider: Rect,
    pub agent_sort_toggle: Rect,
    pub new_tab: Rect,
    pub tab_scroll_left: Rect,
    pub tab_scroll_right: Rect,
    pub workspace_body: Rect,
    pub agent_body: Rect,
    pub workspace_scrollbar: Rect,
    pub agent_scrollbar: Rect,
    pub workspace_scroll_metrics: Option<ScrollMetrics>,
    pub agent_scroll_metrics: Option<ScrollMetrics>,
}

pub const SIDEBAR_MIN_WIDTH: u16 = 18;
pub const SIDEBAR_MAX_WIDTH: u16 = 36;
pub const SIDEBAR_COLLAPSED_WIDTH: u16 = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub sidebar: Rect,
    pub tab_bar: Rect,
    pub pane_surface: Rect,
}

/// Herdr's desktop layout: sidebar on the left, tab bar on top of the panes.
pub fn layout(cols: u16, rows: u16, sidebar_width: u16, collapsed: bool) -> Layout {
    let sidebar_width = if collapsed {
        SIDEBAR_COLLAPSED_WIDTH
    } else {
        sidebar_width.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH)
    }
    .min(cols.saturating_sub(1));
    let tab_height = u16::from(rows > 1);
    let main_width = cols.saturating_sub(sidebar_width);
    Layout {
        sidebar: Rect::new(0, 0, sidebar_width, rows),
        tab_bar: Rect::new(sidebar_width, 0, main_width, tab_height),
        pane_surface: Rect::new(
            sidebar_width,
            tab_height,
            main_width,
            rows.saturating_sub(tab_height),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layout_clamps_the_sidebar_and_puts_tabs_above_panes() {
        let l = layout(140, 40, 26, false);
        assert_eq!(l.sidebar, Rect::new(0, 0, 26, 40));
        assert_eq!(l.tab_bar, Rect::new(26, 0, 114, 1));
        assert_eq!(l.pane_surface, Rect::new(26, 1, 114, 39));
        assert_eq!(layout(140, 40, 80, false).sidebar.width, SIDEBAR_MAX_WIDTH);
        assert_eq!(
            layout(140, 40, 26, true).sidebar.width,
            SIDEBAR_COLLAPSED_WIDTH
        );
    }
}
