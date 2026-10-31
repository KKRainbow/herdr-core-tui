//! Herdr's interface: chrome, overlays, and keymap adapted from Herdr's client shell
//! and fed by `herdr-client-lib` documents and core terminal facts.
pub mod chrome;
pub mod keys;
pub mod mobile;
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
    pub indented: bool,
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
    pub mobile_switch: Rect,
    pub mobile_close: Rect,
    pub mobile_targets: Vec<(Rect, mobile::MobileTarget)>,
    pub mobile_max_scroll: usize,
}

pub const SIDEBAR_MIN_WIDTH: u16 = 18;
pub const SIDEBAR_MAX_WIDTH: u16 = 36;
pub const SIDEBAR_COLLAPSED_WIDTH: u16 = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    /// Herdr's mobile header; empty on wide terminals.
    pub mobile_header: Rect,
    pub sidebar: Rect,
    pub tab_bar: Rect,
    pub pane_surface: Rect,
}

/// Where the tab row goes, from Herdr's `tab_bar_position` and `hide_tab_bar_when_single_tab`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabBar {
    #[default]
    Top,
    Bottom,
    Hidden,
}

/// Herdr's sidebar and tab-row options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub sidebar_width: u16,
    pub collapsed: bool,
    /// Herdr's `sidebar_collapsed_mode = "hidden"` leaves no strip.
    pub hidden_when_collapsed: bool,
    pub min_width: u16,
    pub max_width: u16,
    pub tab_bar: TabBar,
    /// Herdr's `mobile_width_threshold`: at or below this width, the mobile layout.
    pub mobile_width_threshold: u16,
}

impl Default for Shape {
    fn default() -> Self {
        Self {
            sidebar_width: 26,
            collapsed: false,
            hidden_when_collapsed: false,
            min_width: SIDEBAR_MIN_WIDTH,
            max_width: SIDEBAR_MAX_WIDTH,
            tab_bar: TabBar::Top,
            mobile_width_threshold: 64,
        }
    }
}

/// Herdr's desktop layout: sidebar on the left, the tab row above (or below) the panes.
pub fn layout(cols: u16, rows: u16, shape: Shape) -> Layout {
    if cols <= shape.mobile_width_threshold {
        let header_height = rows.min(2);
        return Layout {
            mobile_header: Rect::new(0, 0, cols, header_height),
            sidebar: Rect::default(),
            tab_bar: Rect::default(),
            pane_surface: Rect::new(0, header_height, cols, rows.saturating_sub(header_height)),
        };
    }
    let sidebar_width = if shape.collapsed {
        if shape.hidden_when_collapsed {
            0
        } else {
            SIDEBAR_COLLAPSED_WIDTH
        }
    } else {
        let max = shape.max_width.max(shape.min_width);
        shape.sidebar_width.clamp(shape.min_width, max)
    }
    .min(cols.saturating_sub(1));
    let tab_height = u16::from(rows > 1 && shape.tab_bar != TabBar::Hidden);
    let main_width = cols.saturating_sub(sidebar_width);
    let panes_height = rows.saturating_sub(tab_height);
    let (tab_y, panes_y) = if shape.tab_bar == TabBar::Bottom {
        (panes_height, 0)
    } else {
        (0, tab_height)
    };
    Layout {
        mobile_header: Rect::default(),
        sidebar: Rect::new(0, 0, sidebar_width, rows),
        tab_bar: Rect::new(sidebar_width, tab_y, main_width, tab_height),
        pane_surface: Rect::new(sidebar_width, panes_y, main_width, panes_height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layout_clamps_the_sidebar_and_puts_tabs_above_panes() {
        let shape = Shape::default();
        let l = layout(140, 40, shape);
        assert_eq!(l.sidebar, Rect::new(0, 0, 26, 40));
        assert_eq!(l.tab_bar, Rect::new(26, 0, 114, 1));
        assert_eq!(l.pane_surface, Rect::new(26, 1, 114, 39));
        let wide = Shape {
            sidebar_width: 80,
            ..shape
        };
        assert_eq!(layout(140, 40, wide).sidebar.width, SIDEBAR_MAX_WIDTH);
        let collapsed = Shape {
            collapsed: true,
            ..shape
        };
        assert_eq!(
            layout(140, 40, collapsed).sidebar.width,
            SIDEBAR_COLLAPSED_WIDTH
        );
    }

    #[test]
    fn herdr_options_move_or_hide_the_tab_row_and_sidebar() {
        let shape = Shape::default();
        let bottom = layout(
            100,
            20,
            Shape {
                tab_bar: TabBar::Bottom,
                ..shape
            },
        );
        assert_eq!(bottom.tab_bar, Rect::new(26, 19, 74, 1));
        assert_eq!(bottom.pane_surface, Rect::new(26, 0, 74, 19));
        let hidden = layout(
            100,
            20,
            Shape {
                tab_bar: TabBar::Hidden,
                collapsed: true,
                hidden_when_collapsed: true,
                ..shape
            },
        );
        assert_eq!(hidden.pane_surface, Rect::new(0, 0, 100, 20));
        let narrow = Shape {
            sidebar_width: 10,
            min_width: 12,
            max_width: 30,
            ..shape
        };
        assert_eq!(layout(100, 20, narrow).sidebar.width, 12);
        let phone = layout(50, 24, shape);
        assert_eq!(phone.mobile_header, Rect::new(0, 0, 50, 2));
        assert_eq!(phone.pane_surface, Rect::new(0, 2, 50, 22));
        assert!(phone.sidebar.is_empty() && phone.tab_bar.is_empty());
    }
}
