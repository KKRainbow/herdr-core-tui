//! Keyboard and mouse behavior adapted from Herdr `src/client/shell/{input,mouse,overlay_input,actions}.rs`.
use super::Pressed;
use super::*;
use crate::editor::TextEditor;
use crossterm::event::MouseEvent;
use ratatui::layout::Position;
use shell::{
    chrome,
    overlays::{
        Confirm, ContextMenu, Help, MenuAction, MenuTarget, NavTarget, Navigator, PaneMove, Rename,
        RenameTarget, Settings, SettingsSection, WorktreeCreate, WorktreeEntry, WorktreeOpen,
        WorktreeRemove, global_menu_items, navigator_rows, navigator_selected_index,
        pane_move_destinations,
    },
    palette::{THEME_NAMES, normalized_theme_name},
    snapshot::Status,
};

const RESIZE_STEP: i32 = 2;

fn contains(rect: Rect, column: u16, row: u16) -> bool {
    rect.contains(Position::new(column, row))
}

impl Workspace {
    pub(super) async fn host(&mut self, event: Event) -> Result<bool> {
        match event {
            Event::Key(key) => self.key(key).await,
            Event::Mouse(mouse) => self.mouse(mouse).await,
            Event::Paste(text) => {
                self.paste(&text)?;
                Ok(false)
            }
            Event::FocusLost => {
                self.focused = false;
                for surface in self.surfaces.values() {
                    if let Some(stream) = &surface.stream {
                        stream.focus(false).await?;
                    }
                }
                Ok(false)
            }
            Event::FocusGained => {
                self.focused = true;
                self.activate().await?;
                Ok(false)
            }
            Event::Resize(..) => {
                self.chrome.reveal_focused_tab = true;
                self.chrome.reveal_focused_workspace = true;
                Ok(false)
            }
        }
    }

    pub(super) async fn activate(&mut self) -> Result<()> {
        if !self.focused {
            return Ok(());
        }
        if let Some(id) = self.active_id()
            && let Some(surface) = self.surfaces.get(&id)
            && let Some(stream) = &surface.stream
        {
            stream.resize(surface.wanted.0, surface.wanted.1).await?;
            stream.focus(true).await?;
        }
        Ok(())
    }

    fn overlay_editor(&mut self) -> Option<&mut TextEditor> {
        if let Some(search) = &mut self.copy_search {
            return Some(&mut search.query);
        }
        match self.overlay.as_mut()? {
            Overlay::Rename(r) => Some(&mut r.input),
            Overlay::Help(h) if h.search_focused => Some(&mut h.query),
            Overlay::Navigator(n) if n.search_focused => Some(&mut n.query),
            Overlay::WorktreeCreate(c) if !c.creating => Some(if c.path_focused {
                &mut c.checkout_path
            } else {
                &mut c.branch
            }),
            Overlay::WorktreeOpen(o) if o.search_focused => Some(&mut o.query),
            _ => None,
        }
    }

    fn paste(&mut self, text: &str) -> Result<()> {
        let line = text.lines().next().unwrap_or("").to_owned();
        if let Some(editor) = self.overlay_editor() {
            if editor.insert(&line) {
                self.after_editor_change();
            }
            return Ok(());
        }
        if self.overlay.is_some() {
            return Ok(());
        }
        if let Some(id) = self.active_id()
            && let Some(surface) = self.surfaces.get(&id)
            && let Some(stream) = &surface.stream
        {
            stream.try_input(surface.vt.paste_bytes(text))?;
        }
        Ok(())
    }

    fn after_editor_change(&mut self) {
        match self.overlay.as_mut() {
            Some(Overlay::Navigator(n)) => {
                n.selected = None;
                n.scroll = 0;
            }
            Some(Overlay::WorktreeOpen(o)) => {
                if let Some(index) = o.filtered_indices().first() {
                    o.selected = *index;
                }
            }
            Some(Overlay::WorktreeCreate(c)) => {
                if c.path_focused {
                    c.custom_path = true;
                } else if !c.custom_path {
                    let base = c
                        .checkout_path
                        .rsplit_once('/')
                        .map_or("", |(base, _)| base);
                    let slug = herdr_client_lib::worktrees::slug(c.branch.trim());
                    c.checkout_path = TextEditor::new(&format!("{base}/{slug}"), false);
                }
                c.error = None;
            }
            _ => {}
        }
    }

    fn forward_key(&mut self, key: KeyEvent) -> Result<()> {
        if let Some(id) = self.active_id()
            && let Some(surface) = self.surfaces.get_mut(&id)
        {
            if key.kind != KeyEventKind::Release {
                surface.vt.scroll_to_bottom();
            }
            let bytes = input::key(&mut surface.vt, key);
            if !bytes.is_empty()
                && let Some(stream) = &surface.stream
            {
                stream.try_input(bytes)?;
            }
        }
        Ok(())
    }

    async fn key(&mut self, key: KeyEvent) -> Result<bool> {
        if key.kind == KeyEventKind::Release {
            if self.overlay.is_none() && self.mode == Mode::Terminal {
                self.forward_key(key)?;
            }
            return Ok(false);
        }
        if self.copy_search.is_some() {
            self.copy_search_key(key);
            return Ok(false);
        }
        if self.overlay.is_some() {
            return self.overlay_key(key).await;
        }
        match self.mode {
            Mode::Prefix => {
                self.mode = self.copy_or_terminal();
                if key.code == KeyCode::Esc {
                    return Ok(false);
                }
                if self.keys.is_prefix(&key) {
                    self.forward_key(key)?;
                    return Ok(false);
                }
                if let Some(action) = self.keys.prefixed(&key) {
                    return self.action(&action).await;
                }
                return Ok(false);
            }
            Mode::Navigate => return self.navigate_key(key).await,
            Mode::Resize => {
                self.resize_key(key).await;
                return Ok(false);
            }
            Mode::Copy => {
                if self.keys.is_prefix(&key) {
                    self.mode = Mode::Prefix;
                } else {
                    self.copy_key(key)?;
                }
                return Ok(false);
            }
            Mode::Terminal => {}
        }
        if self.keys.is_prefix(&key) {
            self.mode = Mode::Prefix;
            return Ok(false);
        }
        if let Some(action) = self.keys.direct(&key) {
            return self.action(&action).await;
        }
        self.forward_key(key)?;
        Ok(false)
    }

    fn copy_or_terminal(&self) -> Mode {
        let copying = self
            .active_id()
            .and_then(|id| self.surfaces.get(&id))
            .is_some_and(|s| s.copy.is_some());
        if copying { Mode::Copy } else { Mode::Terminal }
    }

    // ---- actions ------------------------------------------------------------

    /// Runs one of Herdr's keybind actions.
    pub(super) async fn action(&mut self, action: &str) -> Result<bool> {
        let focused_space = self.snapshot.focused_workspace_id.clone();
        let focused_tab = self.snapshot.focused_tab_id.clone();
        let focused_pane = self.view().focused;
        match action {
            "help" => self.overlay = Some(Overlay::Help(Help::default())),
            "settings" => self.open_settings(),
            "open_notification_target" => {
                if let Some(address) = self.notification_target.clone() {
                    self.focus_agent(&address);
                }
            }
            "edit_scrollback" => {
                if let Some(surface) = self.active_id().and_then(|id| self.surfaces.get(&id)) {
                    self.pending_editor = Some(surface.vt.text());
                }
            }
            "clear_pane" => {
                if let Some(surface) = self.active_id().and_then(|id| self.surfaces.get_mut(&id))
                    && !surface.vt.clear()
                {
                    self.toast("the alternate screen cannot be cleared", true);
                }
            }
            "resize_pane_left" | "resize_pane_right" | "resize_pane_up" | "resize_pane_down" => {
                let (vertical, delta) = match action {
                    "resize_pane_left" => (false, -RESIZE_STEP),
                    "resize_pane_right" => (false, RESIZE_STEP),
                    "resize_pane_up" => (true, -RESIZE_STEP),
                    _ => (true, RESIZE_STEP),
                };
                self.resize_focused(vertical, delta).await;
            }
            "detach" => return Ok(true),
            "reload_config" => match self.reload_config() {
                Ok(()) => self.toast("config reloaded", false),
                Err(error) => self.toast(error.to_string(), true),
            },
            "workspace_picker" => {
                self.mode = Mode::Navigate;
                self.chrome.selected_workspace_id = focused_space;
                self.chrome.reveal_focused_workspace = true;
            }
            "goto" => self.overlay = Some(Overlay::Navigator(Navigator::default())),
            "new_workspace" => {
                if self.preferences.ui.prompt_new_workspace_name {
                    self.open_rename(RenameTarget::NewSpace);
                } else {
                    self.run("space.new", json!({})).await;
                    self.invalidate_git();
                }
            }
            "new_worktree" => {
                if let Some(space) = focused_space {
                    self.open_worktree_create(&space).await;
                }
            }
            "open_worktree" => {
                if let Some(space) = focused_space {
                    self.open_worktree_open(&space).await;
                }
            }
            "remove_worktree" => {
                if let Some(space) = focused_space {
                    self.open_worktree_remove(&space);
                }
            }
            "rename_workspace" => {
                if let Some(space) = focused_space {
                    self.open_rename(RenameTarget::Space(space));
                }
            }
            "close_workspace" => {
                if let Some(space) = focused_space {
                    self.request_workspace_close(&space, None).await;
                }
            }
            "previous_workspace" | "next_workspace" => {
                self.cycle_workspace(action == "previous_workspace");
            }
            "previous_agent" | "next_agent" => self.cycle_agent(action == "previous_agent"),
            "new_tab" => {
                if let Some(space) = focused_space {
                    self.new_tab(space).await;
                }
            }
            "rename_tab" => {
                if let Some(tab) = focused_tab {
                    self.open_rename(RenameTarget::Tab(tab));
                }
            }
            "previous_tab" | "next_tab" => self.cycle_tab(action == "previous_tab"),
            "move_tab_previous" | "move_tab_next" => {
                if let Some(tab) = focused_tab {
                    self.move_tab(&tab, action == "move_tab_previous").await;
                }
            }
            "close_tab" => {
                if let Some(tab) = focused_tab {
                    self.request_tab_close(&tab).await;
                }
            }
            "rename_pane" => {
                if let Some(pane) = focused_pane.filter(|_| self.peek.is_none()) {
                    self.open_rename(RenameTarget::Pane(pane));
                }
            }
            "copy_mode" => self.enter_copy_mode(),
            "focus_pane_left" | "focus_pane_down" | "focus_pane_up" | "focus_pane_right" => {
                if let Some(id) = self.neighbor(action.trim_start_matches("focus_pane_")) {
                    self.focus(&id);
                }
            }
            "swap_pane_left" | "swap_pane_down" | "swap_pane_up" | "swap_pane_right" => {
                if let (Some(pane), Some(other)) = (
                    focused_pane,
                    self.neighbor(action.trim_start_matches("swap_pane_")),
                ) {
                    self.run("pane.swap", json!({"id":pane,"other":other}))
                        .await;
                }
            }
            "cycle_pane_next" | "cycle_pane_previous" => {
                self.cycle_pane(action == "cycle_pane_previous");
            }
            "last_pane" => {
                if let Some(previous) = self.previous_pane.clone()
                    && self.document().is_some_and(|d| d.contains(&previous))
                {
                    self.focus(&previous);
                }
            }
            "split_vertical" | "split_horizontal" => {
                if let Some(pane) = focused_pane.filter(|_| self.peek.is_none()) {
                    let axis = if action == "split_vertical" {
                        "row"
                    } else {
                        "column"
                    };
                    self.run("pane.split", json!({"id":pane,"axis":axis})).await;
                }
            }
            "close_pane" => {
                if let Some(pane) = focused_pane.filter(|_| self.peek.is_none()) {
                    self.request_pane_close(&pane).await;
                }
            }
            "zoom" => self.toggle_zoom(),
            "resize_mode" => self.mode = Mode::Resize,
            "toggle_sidebar" => self.toggle_sidebar(),
            action if action.starts_with("command:") => {
                if let Some(command) = action[8..]
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| self.preferences.commands.get(i).cloned())
                {
                    self.run_custom_command(command).await;
                }
            }
            action if action.starts_with("switch_workspace:") => {
                let index: usize = action[17..].parse().unwrap_or(1);
                let entries =
                    chrome::workspace_entries(&self.snapshot, &self.chrome.collapsed_groups);
                if let Some(id) = entries
                    .get(index.saturating_sub(1))
                    .map(|e| self.snapshot.workspaces[e.index].workspace_id.clone())
                {
                    self.focus_container(&id);
                }
            }
            action if action.starts_with("focus_agent:") => {
                let index: usize = action[12..].parse().unwrap_or(1);
                if let Some(address) = chrome::ordered_agents(&self.snapshot, self.config.sort)
                    .get(index.saturating_sub(1))
                    .map(|a| a.address.clone())
                {
                    self.focus_agent(&address);
                }
            }
            action if action.starts_with("switch_tab:") => {
                let index: usize = action[11..].parse().unwrap_or(1);
                if let Some(tab) = self
                    .snapshot
                    .tabs
                    .iter()
                    .filter(|t| {
                        Some(&t.workspace_id) == self.snapshot.focused_workspace_id.as_ref()
                    })
                    .nth(index.saturating_sub(1))
                    .map(|t| t.tab_id.clone())
                {
                    self.focus_container(&tab);
                }
            }
            _ => {}
        }
        Ok(false)
    }

    pub(super) fn toggle_zoom(&mut self) {
        if self.peek.is_some() {
            return;
        }
        let view = self.view_mut();
        let zoomed = if view.zoomed.is_some() {
            None
        } else {
            view.focused.clone()
        };
        view.zoom(zoomed.as_deref());
    }

    fn toggle_sidebar(&mut self) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        self.chrome.reveal_focused_workspace = true;
        self.save_chrome();
    }

    fn cycle_workspace(&mut self, backward: bool) {
        let entries = chrome::workspace_entries(&self.snapshot, &self.chrome.collapsed_groups);
        if entries.is_empty() {
            return;
        }
        let ids: Vec<String> = entries
            .iter()
            .map(|e| self.snapshot.workspaces[e.index].workspace_id.clone())
            .collect();
        let current = self
            .snapshot
            .focused_workspace_id
            .as_ref()
            .and_then(|id| ids.iter().position(|w| w == id))
            .unwrap_or(0);
        let next = if backward {
            (current + ids.len() - 1) % ids.len()
        } else {
            (current + 1) % ids.len()
        };
        self.focus_container(&ids[next].clone());
    }

    fn cycle_agent(&mut self, backward: bool) {
        let agents: Vec<(String, String)> =
            chrome::ordered_agents(&self.snapshot, self.config.sort)
                .iter()
                .map(|a| (a.pane_id.clone(), a.address.clone()))
                .collect();
        if agents.is_empty() {
            return;
        }
        let current = self
            .snapshot
            .agents
            .iter()
            .find(|a| a.focused)
            .and_then(|focused| agents.iter().position(|(id, _)| *id == focused.pane_id));
        let next = match (current, backward) {
            (Some(i), true) => (i + agents.len() - 1) % agents.len(),
            (Some(i), false) => (i + 1) % agents.len(),
            (None, true) => agents.len() - 1,
            (None, false) => 0,
        };
        let address = agents[next].1.clone();
        self.focus_agent(&address);
    }

    fn cycle_tab(&mut self, backward: bool) {
        let tabs: Vec<String> = self
            .snapshot
            .tabs
            .iter()
            .filter(|t| Some(&t.workspace_id) == self.snapshot.focused_workspace_id.as_ref())
            .map(|t| t.tab_id.clone())
            .collect();
        let Some(current) = tabs
            .iter()
            .position(|t| Some(t) == self.snapshot.focused_tab_id.as_ref())
        else {
            return;
        };
        let next = if backward {
            (current + tabs.len() - 1) % tabs.len()
        } else {
            (current + 1) % tabs.len()
        };
        self.focus_container(&tabs[next].clone());
    }

    async fn move_tab(&mut self, tab: &str, backward: bool) {
        let Some(index) = self
            .snapshot
            .tabs
            .iter()
            .filter(|t| Some(&t.workspace_id) == self.snapshot.focused_workspace_id.as_ref())
            .position(|t| t.tab_id == tab)
        else {
            return;
        };
        let index = if backward {
            index.saturating_sub(1)
        } else {
            index + 1
        };
        self.run("tab.move", json!({"id":tab,"index":index})).await;
    }

    fn cycle_pane(&mut self, backward: bool) {
        let view = self.view();
        let Some(doc) = self.document() else {
            return;
        };
        let Some(tab) = view.active_tab.as_deref().and_then(|id| doc.node(id)) else {
            return;
        };
        let ids = layout::navigation_ids(&tab["root"]);
        if ids.is_empty() {
            return;
        }
        let index = ids
            .iter()
            .position(|id| Some(id) == view.focused.as_ref())
            .unwrap_or(0);
        let step = if backward { ids.len() - 1 } else { 1 };
        self.focus(&ids[(index + step) % ids.len()].clone());
    }

    /// The nearest visible pane in a direction from the focused one.
    fn neighbor(&self, direction: &str) -> Option<String> {
        let current = self
            .active_id()
            .and_then(|id| self.panes.iter().find(|p| p.id == id))?;
        let center = |p: &Pane| {
            (
                i32::from(p.area.x) + i32::from(p.area.width) / 2,
                i32::from(p.area.y) + i32::from(p.area.height) / 2,
            )
        };
        let (x, y) = center(current);
        self.panes
            .iter()
            .filter(|p| p.id != current.id && !p.id.starts_with('@'))
            .filter(|p| {
                let (px, py) = center(p);
                match direction {
                    "left" => px < x,
                    "right" => px > x,
                    "up" => py < y,
                    _ => py > y,
                }
            })
            .min_by_key(|p| {
                let (px, py) = center(p);
                (px - x).abs() + (py - y).abs()
            })
            .map(|p| p.id.clone())
    }

    fn open_rename(&mut self, target: RenameTarget) {
        let (title, text) = match &target {
            RenameTarget::Space(id) => (
                "rename workspace",
                self.snapshot
                    .workspace(id)
                    .map(|w| w.label.clone())
                    .unwrap_or_default(),
            ),
            RenameTarget::Tab(id) => (
                "rename tab",
                self.snapshot
                    .tab(id)
                    .map(|t| t.label.clone())
                    .unwrap_or_default(),
            ),
            RenameTarget::Pane(id) => (
                "rename pane",
                self.snapshot
                    .pane(id)
                    .and_then(|p| p.label.clone())
                    .unwrap_or_default(),
            ),
            RenameTarget::NewSpace => ("new workspace", String::new()),
            RenameTarget::NewTab(space) => (
                "new tab",
                (self
                    .snapshot
                    .tabs
                    .iter()
                    .filter(|t| &t.workspace_id == space)
                    .count()
                    + 1)
                .to_string(),
            ),
        };
        let replace = match &target {
            RenameTarget::NewTab(_) => true,
            RenameTarget::Pane(_) => text.is_empty(),
            _ => false,
        };
        self.overlay = Some(Overlay::Rename(Rename {
            title,
            target,
            input: TextEditor::new(&text, replace),
        }));
    }

    async fn submit_rename(&mut self, mut rename: Rename) {
        rename.input.trim_and_accept();
        let name = rename.input.as_str().to_owned();
        match rename.target {
            RenameTarget::NewSpace => {
                self.run("space.new", json!({"name":name})).await;
                self.invalidate_git();
            }
            RenameTarget::NewTab(space) => {
                let number = (self
                    .snapshot
                    .tabs
                    .iter()
                    .filter(|t| t.workspace_id == space)
                    .count()
                    + 1)
                .to_string();
                let name = if name == number { String::new() } else { name };
                self.run("tab.new", json!({"space":space,"name":name}))
                    .await;
            }
            RenameTarget::Space(id) => {
                self.run("space.rename", json!({"id":id,"name":name})).await;
            }
            RenameTarget::Tab(id) => {
                let number = self.snapshot.tab(&id).map(|t| t.number.to_string());
                let name = if Some(&name) == number.as_ref() {
                    String::new()
                } else {
                    name
                };
                self.run("tab.rename", json!({"id":id,"name":name})).await;
            }
            RenameTarget::Pane(id) => {
                self.run("pane.rename", json!({"id":id,"name":name})).await;
            }
        }
    }

    // ---- closing ------------------------------------------------------------

    fn pane_count(&self, spaces: &[&str]) -> usize {
        self.snapshot
            .panes
            .iter()
            .filter(|p| spaces.contains(&p.workspace_id.as_str()))
            .count()
    }

    async fn confirm(&mut self, title: String, detail: String, command: &str, id: &str) {
        let Some(review) = self.run(command, json!({"id":id})).await else {
            return;
        };
        self.ask(Confirm {
            title,
            detail,
            command: command.into(),
            args: json!({"id":id,"confirm":true,"expected":review}),
        })
        .await;
    }

    /// Shows a reviewed close, or runs it at once with Herdr's `confirm_close = false`.
    async fn ask(&mut self, confirm: Confirm) {
        if self.preferences.ui.confirm_close {
            self.overlay = Some(Overlay::Confirm(confirm));
        } else {
            self.accept_confirm(confirm).await;
        }
    }

    /// Herdr's `prompt_new_tab_name`: ask for a name first, else create it at once.
    async fn new_tab(&mut self, space: String) {
        if self.preferences.ui.prompt_new_tab_name {
            self.open_rename(RenameTarget::NewTab(space));
        } else {
            self.run("tab.new", json!({"space":space})).await;
        }
    }

    async fn request_workspace_close(&mut self, space: &str, group: Option<bool>) {
        let Some(workspace) = self.snapshot.workspace(space).cloned() else {
            return;
        };
        let group =
            group.unwrap_or_else(|| chrome::workspace_close_is_group(&self.snapshot, &workspace));
        let members: Vec<String> = match workspace.worktree.as_ref().filter(|_| group) {
            Some(worktree) => self
                .snapshot
                .workspaces
                .iter()
                .filter(|w| w.worktree.as_ref().is_some_and(|t| t.key == worktree.key))
                .map(|w| w.workspace_id.clone())
                .collect(),
            None => vec![space.to_owned()],
        };
        let refs: Vec<&str> = members.iter().map(String::as_str).collect();
        let count = self.pane_count(&refs);
        let panes = if count == 1 {
            "1 pane".to_owned()
        } else {
            format!("{count} panes")
        };
        if members.len() > 1 {
            let args = json!({"ids":members});
            let Some(review) = self.run("space.close.group", args.clone()).await else {
                return;
            };
            let detail = format!(
                "{} — {} workspaces, {panes}",
                workspace.label,
                members.len()
            );
            self.ask(Confirm {
                title: "Close worktree group?".into(),
                detail,
                command: "space.close.group".into(),
                args: json!({"ids":members,"confirm":true,"expected":review}),
            })
            .await;
        } else {
            self.confirm(
                "Close workspace?".into(),
                format!("{} — {panes}", workspace.label),
                "space.close",
                space,
            )
            .await;
        }
    }

    async fn request_tab_close(&mut self, tab: &str) {
        let Some(space) = self.snapshot.tab(tab).map(|t| t.workspace_id.clone()) else {
            return;
        };
        let last = !self
            .snapshot
            .tabs
            .iter()
            .any(|t| t.workspace_id == space && t.tab_id != tab);
        if last {
            self.request_workspace_close(&space, Some(false)).await;
        } else if self
            .run("tab.close", json!({"id":tab,"confirm":true}))
            .await
            .is_some()
        {
            self.focus_container(&space);
        }
    }

    async fn request_pane_close(&mut self, pane: &str) {
        let Some(info) = self.snapshot.pane(pane).cloned() else {
            return;
        };
        let last_pane = !self
            .snapshot
            .panes
            .iter()
            .any(|p| p.tab_id == info.tab_id && p.pane_id != pane);
        let last_tab = !self
            .snapshot
            .tabs
            .iter()
            .any(|t| t.workspace_id == info.workspace_id && t.tab_id != info.tab_id);
        if last_pane && last_tab {
            self.request_workspace_close(&info.workspace_id, Some(false))
                .await;
        } else if self
            .run("pane.close", json!({"id":pane,"confirm":true}))
            .await
            .is_some()
        {
            let target = if last_pane {
                &info.workspace_id
            } else {
                &info.tab_id
            };
            self.focus_container(&target.clone());
        }
    }

    async fn accept_confirm(&mut self, confirm: Confirm) {
        self.run(&confirm.command, confirm.args).await;
        self.invalidate_git();
    }

    fn open_pane_move(&mut self, pane: String) {
        let destination = pane_move_destinations(&self.snapshot, &pane)
            .first()
            .map(|t| t.tab_id.clone())
            .unwrap_or_default();
        self.overlay = Some(Overlay::PaneMove(PaneMove {
            pane_id: pane,
            machine: self.machine.clone(),
            destination,
            error: None,
        }));
    }

    async fn submit_pane_move(&mut self, mut picker: PaneMove) {
        if picker.machine != self.machine {
            picker.error = Some("The machine changed. Select the source pane again.".into());
        } else if !pane_move_destinations(&self.snapshot, &picker.pane_id)
            .iter()
            .any(|tab| tab.tab_id == picker.destination)
        {
            picker.error = Some("Select an available destination tab.".into());
        } else {
            match self
                .command(
                    "pane.move",
                    json!({"id":picker.pane_id,"destination":picker.destination}),
                )
                .await
            {
                Ok(_) => {
                    self.view_mut().zoom(None);
                    self.focus(&picker.pane_id);
                    return;
                }
                Err(error) => picker.error = Some(error.to_string()),
            }
        }
        self.overlay = Some(Overlay::PaneMove(picker));
    }

    fn step_pane_destination(&self, picker: &mut PaneMove, delta: isize) {
        let tabs = pane_move_destinations(&self.snapshot, &picker.pane_id);
        if tabs.is_empty() {
            return;
        }
        let index = tabs
            .iter()
            .position(|tab| tab.tab_id == picker.destination)
            .map_or(0, |index| {
                (index as isize + delta).clamp(0, tabs.len() as isize - 1) as usize
            });
        picker.destination = tabs[index].tab_id.clone();
        picker.error = None;
    }

    // ---- worktrees ----------------------------------------------------------

    fn git(&self) -> herdr_client_lib::worktrees::Git {
        herdr_client_lib::worktrees::Git {
            client: self.session.client.clone(),
            machine: self.machine.clone(),
            deadline: TIME,
        }
    }

    /// The main checkout of a space's repository, which owns its worktrees.
    fn repository(&self, space: &str) -> Option<String> {
        let workspace = self.snapshot.workspace(space)?;
        let worktree = workspace.worktree.as_ref()?;
        Some(match worktree.key.strip_suffix("/.git") {
            Some(main) => main.to_owned(),
            None => workspace.repository.clone()?,
        })
    }

    async fn open_worktree_create(&mut self, space: &str) {
        let Some(repository) = self.repository(space) else {
            self.toast("not a git repository", true);
            return;
        };
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let adjectives = [
            "brave", "calm", "clear", "green", "lucky", "quiet", "rapid", "silver",
        ];
        let nouns = [
            "river", "cloud", "field", "forest", "harbor", "meadow", "stone", "valley",
        ];
        let branch = format!(
            "worktree/{}-{}-{:04x}",
            adjectives[(seed % 8) as usize],
            nouns[((seed / 8) % 8) as usize],
            seed & 0xffff
        );
        let directory = self.preferences.worktrees.directory.clone();
        let checkout_path = match self
            .git()
            .default_path_in(&repository, &branch, directory.as_deref())
            .await
        {
            Ok(path) => path,
            Err(error) => {
                self.toast(error.to_string(), true);
                return;
            }
        };
        self.overlay = Some(Overlay::WorktreeCreate(WorktreeCreate {
            repository,
            branch: TextEditor::new(&branch, false),
            new_branch: true,
            checkout_path: TextEditor::new(&checkout_path, false),
            path_focused: false,
            custom_path: false,
            creating: false,
            error: None,
        }));
    }

    async fn open_worktree_open(&mut self, space: &str) {
        let Some(repository) = self.repository(space) else {
            self.toast("not a git repository", true);
            return;
        };
        let checkouts = match self.git().list(&repository).await {
            Ok(list) => list,
            Err(error) => {
                self.toast(error.to_string(), true);
                return;
            }
        };
        let entries: Vec<WorktreeEntry> = checkouts
            .into_iter()
            .filter(|c| !c.bare && !c.prunable)
            .enumerate()
            .map(|(index, c)| WorktreeEntry {
                open_workspace_id: self
                    .snapshot
                    .workspaces
                    .iter()
                    .find(|w| w.repository.as_deref() == Some(c.path.as_str()))
                    .map(|w| w.workspace_id.clone()),
                label: c
                    .branch
                    .clone()
                    .unwrap_or_else(|| snapshot::folder_label(&c.path, "")),
                branch: c.branch,
                is_linked_worktree: index > 0,
                path: c.path,
            })
            .collect();
        let selected = entries
            .iter()
            .position(|e| e.open_workspace_id.is_none())
            .unwrap_or(0);
        self.overlay = Some(Overlay::WorktreeOpen(WorktreeOpen {
            entries,
            selected,
            query: TextEditor::default(),
            search_focused: false,
            error: None,
            opening: false,
        }));
    }

    fn open_worktree_remove(&mut self, space: &str) {
        let Some(workspace) = self.snapshot.workspace(space) else {
            return;
        };
        let linked = workspace
            .worktree
            .as_ref()
            .is_some_and(|w| w.is_linked_worktree);
        let (Some(repository), Some(path), true) =
            (self.repository(space), workspace.repository.clone(), linked)
        else {
            self.toast("not a linked worktree checkout", true);
            return;
        };
        self.overlay = Some(Overlay::WorktreeRemove(WorktreeRemove {
            workspace_id: space.into(),
            repository,
            path,
            error: None,
            removing: false,
            review: None,
            force_confirmation: false,
        }));
    }

    async fn submit_worktree_create(&mut self, mut create: WorktreeCreate) {
        let branch = create.branch.trim().to_owned();
        if branch.is_empty() {
            create.error = Some("branch name is required".into());
            self.overlay = Some(Overlay::WorktreeCreate(create));
            return;
        }
        if create.checkout_path.trim().is_empty() {
            create.error = Some("checkout path is required".into());
            self.overlay = Some(Overlay::WorktreeCreate(create));
            return;
        }
        let args = json!({"repository":create.repository,"branch":branch,"path":create.checkout_path.as_str(),"new_branch":create.new_branch});
        match self.command("worktree.new", args).await {
            Ok(_) => self.invalidate_git(),
            Err(error) => {
                create.error = Some(error.to_string());
                self.overlay = Some(Overlay::WorktreeCreate(create));
            }
        }
    }

    async fn submit_worktree_open(&mut self, mut open: WorktreeOpen) {
        let Some(entry) = open.selected_entry_index().map(|i| open.entries[i].clone()) else {
            return;
        };
        if let Some(space) = &entry.open_workspace_id {
            self.focus_container(&space.clone());
            return;
        }
        match self
            .command("worktree.open", json!({"path":entry.path}))
            .await
        {
            Ok(_) => self.invalidate_git(),
            Err(error) => {
                open.error = Some(error.to_string());
                self.overlay = Some(Overlay::WorktreeOpen(open));
            }
        }
    }

    async fn submit_worktree_remove(&mut self, mut remove: WorktreeRemove) {
        let args =
            json!({"repository":remove.repository,"path":remove.path,"id":remove.workspace_id});
        let review = match &remove.review {
            Some(review) => review.clone(),
            None => match self.command("worktree.remove", args.clone()).await {
                Ok(review) => review,
                Err(error) => {
                    remove.error = Some(error.to_string());
                    self.overlay = Some(Overlay::WorktreeRemove(remove));
                    return;
                }
            },
        };
        // Herdr asks once more when removal deletes work the person has not accepted.
        let risky = review["risks"].as_array().is_some_and(|r| !r.is_empty());
        if risky && !remove.force_confirmation {
            remove.force_confirmation = true;
            remove.review = Some(review);
            remove.error = None;
            self.overlay = Some(Overlay::WorktreeRemove(remove));
            return;
        }
        let mut confirmed = args;
        confirmed["confirm"] = json!(true);
        confirmed["expected"] = review;
        confirmed["force"] = json!(risky);
        match self.command("worktree.remove", confirmed).await {
            Ok(_) => self.invalidate_git(),
            Err(error) => {
                remove.review = None;
                remove.force_confirmation = false;
                remove.error = Some(error.to_string());
                self.overlay = Some(Overlay::WorktreeRemove(remove));
            }
        }
    }

    // ---- custom commands (Herdr `src/app/custom_commands.rs`) --------------

    /// What a custom command learns about the viewer, under Herdr's variable names.
    fn custom_command_env(&self) -> (BTreeMap<String, String>, Option<String>) {
        let view = self.view();
        let mut env = BTreeMap::new();
        if let Some((session, _)) = self.view_key.rsplit_once(':') {
            env.insert("HC_SESSION".to_owned(), session.to_owned());
        }
        for (name, value) in [
            ("HERDR_ACTIVE_WORKSPACE_ID", &view.active_space),
            ("HERDR_ACTIVE_TAB_ID", &view.active_tab),
            ("HERDR_ACTIVE_PANE_ID", &view.focused),
        ] {
            if let Some(value) = value {
                env.insert(name.to_owned(), value.clone());
            }
        }
        let cwd = self
            .active_id()
            .and_then(|id| self.surfaces.get(&id))
            .and_then(|s| {
                self.session
                    .terminals
                    .values()
                    .find(|r| r.address == s.address)
            })
            .and_then(|r| r.folder())
            .map(str::to_owned);
        if let Some(cwd) = &cwd {
            env.insert("HERDR_ACTIVE_PANE_CWD".to_owned(), cwd.clone());
        }
        (env, cwd)
    }

    async fn run_custom_command(&mut self, command: crate::preferences::Command) {
        use crate::preferences::CommandKind;
        let (env, cwd) = self.custom_command_env();
        match command.kind {
            CommandKind::Shell => {
                let mut process = std::process::Command::new("/bin/sh");
                process
                    .args(["-lc", &command.command])
                    .envs(&env)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                // The folder belongs to the terminal's machine; use it only when it is here too.
                if let Some(cwd) =
                    cwd.filter(|c| self.machine.is_empty() && std::path::Path::new(c).is_dir())
                {
                    process.current_dir(cwd);
                }
                match process.spawn() {
                    Ok(mut child) => {
                        std::thread::spawn(move || child.wait());
                    }
                    Err(error) => self.toast(error.to_string(), true),
                }
            }
            CommandKind::Pane | CommandKind::Popup => {
                let Some(pane) = self.view().focused.filter(|_| self.peek.is_none()) else {
                    return self.toast("custom pane commands need a focused pane", true);
                };
                if self
                    .run(
                        "pane.split",
                        json!({"id":pane,"axis":"row","line":command.command,"env":env}),
                    )
                    .await
                    .is_some()
                {
                    let view = self.view_mut();
                    let focused = view.focused.clone();
                    view.zoom(focused.as_deref());
                }
            }
            CommandKind::PluginAction => {
                self.toast("plugin actions are not supported by herder-tui", true);
            }
        }
    }

    // ---- settings (Herdr `src/client/shell/settings.rs`) ---------------------

    fn open_settings(&mut self) {
        let original_theme = self.preferences.theme.clone();
        let mut settings = Settings {
            section: SettingsSection::Theme,
            selected: 0,
            original_theme,
        };
        settings.selected = self.settings_current(SettingsSection::Theme);
        self.overlay = Some(Overlay::Settings(settings));
    }

    fn settings_current(&self, section: SettingsSection) -> usize {
        match section {
            SettingsSection::Theme => {
                let theme = normalized_theme_name(&self.preferences.theme);
                THEME_NAMES
                    .iter()
                    .position(|name| normalized_theme_name(name) == theme)
                    .unwrap_or(0)
            }
            SettingsSection::Indicators => usize::from(self.config.symbols),
            SettingsSection::Sound => usize::from(!self.preferences.sound.enabled),
        }
    }

    fn select_settings_section(&mut self, settings: &mut Settings, section: SettingsSection) {
        settings.section = section;
        settings.selected = self.settings_current(section);
    }

    fn move_settings_selection(&mut self, settings: &mut Settings, delta: isize) {
        let count = settings.section.choices();
        settings.selected = (settings.selected as isize + delta)
            .clamp(0, count.saturating_sub(1) as isize) as usize;
        self.preview_settings(settings);
    }

    /// Shows a highlighted theme at once; Esc goes back to the saved one.
    fn preview_settings(&mut self, settings: &Settings) {
        if settings.section == SettingsSection::Theme
            && let Some(name) = THEME_NAMES.get(settings.selected)
        {
            self.config.palette = self.preferences.palette(name);
        }
    }

    fn cancel_settings(&mut self, settings: Settings) {
        self.config.palette = self.preferences.palette(&settings.original_theme);
    }

    fn apply_settings(&mut self, mut settings: Settings) {
        use crate::config_edit::Edit;
        let edit = match settings.section {
            SettingsSection::Theme => {
                let Some(name) = THEME_NAMES.get(settings.selected) else {
                    return;
                };
                Edit::Theme(name)
            }
            SettingsSection::Indicators if settings.selected == 0 => Edit::StatusIndicators("dots"),
            SettingsSection::Indicators => Edit::StatusIndicators("symbols"),
            SettingsSection::Sound => Edit::Sound(settings.selected == 0),
        };
        let saved = crate::preferences::edit(edit).and_then(|()| self.reload_config());
        match saved {
            Err(error) => {
                self.toast(error.to_string(), true);
                self.overlay = Some(Overlay::Settings(settings));
            }
            // A chosen theme closes settings, as in Herdr; other choices stay open.
            Ok(()) if settings.section == SettingsSection::Theme => {}
            Ok(()) => {
                settings.original_theme = self.preferences.theme.clone();
                self.overlay = Some(Overlay::Settings(settings));
            }
        }
    }

    fn settings_key(&mut self, mut settings: Settings, key: KeyEvent) {
        let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
        let step = |settings: &Settings, delta: isize| {
            let all = SettingsSection::ALL;
            let current = all.iter().position(|s| *s == settings.section).unwrap_or(0);
            all[(current as isize + delta).rem_euclid(all.len() as isize) as usize]
        };
        match key.code {
            KeyCode::Esc => return self.cancel_settings(settings),
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') if plain => {
                let next = step(&settings, 1);
                self.select_settings_section(&mut settings, next);
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') if plain => {
                let previous = step(&settings, -1);
                self.select_settings_section(&mut settings, previous);
            }
            KeyCode::Up | KeyCode::Char('k') if plain => {
                self.move_settings_selection(&mut settings, -1)
            }
            KeyCode::Down | KeyCode::Char('j') if plain => {
                self.move_settings_selection(&mut settings, 1)
            }
            KeyCode::Enter | KeyCode::Char(' ') if plain => return self.apply_settings(settings),
            _ => {}
        }
        self.overlay = Some(Overlay::Settings(settings));
    }

    // ---- overlay keys -------------------------------------------------------

    async fn overlay_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(overlay) = self.overlay.take() else {
            return Ok(false);
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let up = matches!(key.code, KeyCode::Up) || (ctrl && key.code == KeyCode::Char('p'));
        let down = matches!(key.code, KeyCode::Down) || (ctrl && key.code == KeyCode::Char('n'));
        match overlay {
            Overlay::GlobalMenu { highlighted } => {
                let count = global_menu_items().len();
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter => {
                        return self
                            .menu_action(global_menu_items()[highlighted].1, None)
                            .await;
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.overlay = Some(Overlay::GlobalMenu {
                            highlighted: highlighted.saturating_sub(1),
                        })
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        self.overlay = Some(Overlay::GlobalMenu {
                            highlighted: (highlighted + 1).min(count - 1),
                        })
                    }
                    _ => self.overlay = Some(Overlay::GlobalMenu { highlighted }),
                }
            }
            Overlay::ContextMenu(mut menu) => {
                let items = menu.items();
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter => {
                        if let Some((_, action)) = items.get(menu.highlighted) {
                            return self.menu_action(*action, Some(menu.target)).await;
                        }
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        menu.highlighted = menu.highlighted.saturating_sub(1);
                        self.overlay = Some(Overlay::ContextMenu(menu));
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        menu.highlighted =
                            (menu.highlighted + 1).min(items.len().saturating_sub(1));
                        self.overlay = Some(Overlay::ContextMenu(menu));
                    }
                    _ => self.overlay = Some(Overlay::ContextMenu(menu)),
                }
            }
            Overlay::Rename(mut rename) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => self.submit_rename(rename).await,
                KeyCode::Char('c') if ctrl => {
                    rename.input.clear();
                    self.overlay = Some(Overlay::Rename(rename));
                }
                _ => {
                    rename.input.handle_key(&key);
                    self.overlay = Some(Overlay::Rename(rename));
                }
            },
            Overlay::Confirm(confirm) => match key.code {
                KeyCode::Enter | KeyCode::Char('y') => self.accept_confirm(confirm).await,
                KeyCode::Esc | KeyCode::Char('n') => {}
                _ => self.overlay = Some(Overlay::Confirm(confirm)),
            },
            Overlay::PaneMove(mut picker) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => self.submit_pane_move(picker).await,
                _ => {
                    if up || key.code == KeyCode::Char('k') {
                        self.step_pane_destination(&mut picker, -1);
                    }
                    if down || key.code == KeyCode::Char('j') {
                        self.step_pane_destination(&mut picker, 1);
                    }
                    self.overlay = Some(Overlay::PaneMove(picker));
                }
            },
            Overlay::Help(mut help) => {
                let max = self.overlay_render.help_max_scroll;
                let mut close = false;
                if help.search_focused {
                    match key.code {
                        KeyCode::Esc => help.search_focused = false,
                        KeyCode::Up => help.scroll = help.scroll.saturating_sub(1),
                        KeyCode::Down => help.scroll = (help.scroll + 1).min(max),
                        KeyCode::Enter => help.search_focused = false,
                        _ => {
                            if help.query.handle_key(&key).is_some() {
                                help.scroll = 0;
                            }
                        }
                    }
                } else {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => close = true,
                        KeyCode::Char('/') => help.search_focused = true,
                        KeyCode::Char('k') | KeyCode::Up => {
                            help.scroll = help.scroll.saturating_sub(1)
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            help.scroll = (help.scroll + 1).min(max)
                        }
                        KeyCode::PageUp => help.scroll = help.scroll.saturating_sub(10),
                        KeyCode::PageDown => help.scroll = (help.scroll + 10).min(max),
                        _ => {}
                    }
                }
                if !close {
                    self.overlay = Some(Overlay::Help(help));
                }
            }
            Overlay::Navigator(navigator) => {
                return self.navigator_key(navigator, key, up, down).await;
            }
            Overlay::WorktreeCreate(mut create) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter if !create.creating => self.submit_worktree_create(create).await,
                KeyCode::Tab | KeyCode::BackTab if !create.creating => {
                    create.new_branch = !create.new_branch;
                    create.error = None;
                    self.overlay = Some(Overlay::WorktreeCreate(create));
                }
                KeyCode::Up | KeyCode::Down if !create.creating => {
                    create.path_focused = key.code == KeyCode::Down;
                    self.overlay = Some(Overlay::WorktreeCreate(create));
                }
                _ => {
                    self.overlay = Some(Overlay::WorktreeCreate(create));
                    if self
                        .overlay_editor()
                        .and_then(|editor| editor.handle_key(&key))
                        == Some(true)
                    {
                        self.after_editor_change();
                    }
                }
            },
            Overlay::WorktreeOpen(mut open) => {
                let filtered = open.filtered_indices();
                let position = filtered.iter().position(|i| *i == open.selected);
                let step = |delta: isize| {
                    let next = position.map_or(0, |p| {
                        (p as isize + delta).clamp(0, filtered.len() as isize - 1) as usize
                    });
                    filtered.get(next).copied()
                };
                match key.code {
                    KeyCode::Esc if open.search_focused => {
                        open.search_focused = false;
                        self.overlay = Some(Overlay::WorktreeOpen(open));
                    }
                    KeyCode::Esc => {}
                    KeyCode::Enter => self.submit_worktree_open(open).await,
                    _ if up || (!open.search_focused && key.code == KeyCode::Char('k')) => {
                        if let Some(next) = step(-1) {
                            open.selected = next;
                        }
                        self.overlay = Some(Overlay::WorktreeOpen(open));
                    }
                    _ if down || (!open.search_focused && key.code == KeyCode::Char('j')) => {
                        if let Some(next) = step(1) {
                            open.selected = next;
                        }
                        self.overlay = Some(Overlay::WorktreeOpen(open));
                    }
                    KeyCode::Char('/') if !open.search_focused => {
                        open.search_focused = true;
                        self.overlay = Some(Overlay::WorktreeOpen(open));
                    }
                    _ => {
                        let changed = open.search_focused && open.query.handle_key(&key).is_some();
                        self.overlay = Some(Overlay::WorktreeOpen(open));
                        if changed {
                            self.after_editor_change();
                        }
                    }
                }
            }
            Overlay::WorktreeRemove(remove) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter if !remove.removing => self.submit_worktree_remove(remove).await,
                _ => self.overlay = Some(Overlay::WorktreeRemove(remove)),
            },
            Overlay::Settings(settings) => self.settings_key(settings, key),
        }
        Ok(false)
    }

    fn navigator_rows(&self, navigator: &Navigator) -> Vec<shell::overlays::NavRow> {
        let terminals = &self.session.terminals;
        let cwd = |address: &str| {
            terminals
                .values()
                .find(|r| r.address == address)
                .and_then(snapshot::cwd)
                .map(str::to_owned)
        };
        navigator_rows(
            &self.snapshot,
            &self.machines,
            &self.machine,
            &cwd,
            navigator,
        )
    }

    async fn navigator_key(
        &mut self,
        mut navigator: Navigator,
        key: KeyEvent,
        up: bool,
        down: bool,
    ) -> Result<bool> {
        let rows = self.navigator_rows(&navigator);
        let selected = navigator_selected_index(&rows, &navigator);
        let select = |navigator: &mut Navigator, index: usize| {
            navigator.selected = rows.get(index).map(|r| r.target.clone());
        };
        let plain = !navigator.search_focused;
        match key.code {
            KeyCode::Esc if navigator.search_focused => navigator.search_focused = false,
            KeyCode::Esc => return Ok(false),
            KeyCode::Enter => {
                if let Some(target) = selected.and_then(|i| rows.get(i)).map(|r| r.target.clone()) {
                    self.open_nav_target(target).await;
                }
                return Ok(false);
            }
            _ if up || (plain && key.code == KeyCode::Char('k')) => {
                if let Some(index) = selected {
                    select(&mut navigator, index.saturating_sub(1));
                }
            }
            _ if down || (plain && key.code == KeyCode::Char('j')) => {
                if let Some(index) = selected {
                    select(
                        &mut navigator,
                        (index + 1).min(rows.len().saturating_sub(1)),
                    );
                }
            }
            KeyCode::Left | KeyCode::Right if plain => {
                let forward = key.code == KeyCode::Right;
                let start = selected.unwrap_or(0);
                let found = if forward {
                    (start + 1..rows.len())
                        .find(|i| matches!(rows[*i].target, NavTarget::Workspace(_)))
                } else {
                    (0..start)
                        .rev()
                        .find(|i| matches!(rows[*i].target, NavTarget::Workspace(_)))
                };
                if let Some(index) = found {
                    select(&mut navigator, index);
                }
            }
            KeyCode::Char('/') if plain => navigator.search_focused = true,
            KeyCode::Char(c @ ('a' | 'b' | 'w' | 'i' | 'd')) if plain => {
                navigator.filter = match c {
                    'b' => Some(Status::Blocked),
                    'w' => Some(Status::Working),
                    'i' => Some(Status::Idle),
                    'd' => Some(Status::Done),
                    _ => None,
                };
                navigator.selected = None;
            }
            _ if navigator.search_focused && navigator.query.handle_key(&key).is_some() => {
                navigator.selected = None;
                navigator.scroll = 0;
            }
            _ => {}
        }
        self.overlay = Some(Overlay::Navigator(navigator));
        Ok(false)
    }

    async fn open_nav_target(&mut self, target: NavTarget) {
        match target {
            NavTarget::Machine(machine) => {
                self.run("machine", json!({"machine":machine})).await;
            }
            NavTarget::Workspace(space) => self.focus_container(&space),
            NavTarget::Pane(pane) => self.focus(&pane),
            NavTarget::Terminal(address) => self.focus_agent(&address),
            NavTarget::Loose => {}
        }
    }

    async fn menu_action(
        &mut self,
        action: MenuAction,
        target: Option<MenuTarget>,
    ) -> Result<bool> {
        use MenuAction as A;
        match (action, target) {
            (A::Keybinds, _) => return self.action("help").await,
            (A::Settings, _) => return self.action("settings").await,
            (A::ReloadConfig, _) => return self.action("reload_config").await,
            (A::Detach, _) => return Ok(true),
            (A::Rename, Some(MenuTarget::Workspace { workspace_id, .. })) => {
                self.open_rename(RenameTarget::Space(workspace_id))
            }
            (
                A::Close,
                Some(MenuTarget::Workspace {
                    workspace_id,
                    close_group,
                    ..
                }),
            ) => {
                self.request_workspace_close(&workspace_id, Some(close_group))
                    .await
            }
            (A::NewWorktree, Some(MenuTarget::Workspace { workspace_id, .. })) => {
                self.open_worktree_create(&workspace_id).await
            }
            (A::OpenWorktree, Some(MenuTarget::Workspace { workspace_id, .. })) => {
                self.open_worktree_open(&workspace_id).await
            }
            (A::RemoveWorktree, Some(MenuTarget::Workspace { workspace_id, .. })) => {
                self.open_worktree_remove(&workspace_id)
            }
            (A::ToggleGroup, Some(MenuTarget::Workspace { workspace_id, .. })) => {
                if let Some(key) = self
                    .snapshot
                    .workspace(&workspace_id)
                    .and_then(|w| w.worktree.as_ref())
                    .map(|w| w.key.clone())
                {
                    self.toggle_group(key);
                }
            }
            (A::NewTab, Some(MenuTarget::Tab { tab_id })) => {
                if let Some(space) = self.snapshot.tab(&tab_id).map(|t| t.workspace_id.clone()) {
                    self.new_tab(space).await;
                }
            }
            (A::Rename, Some(MenuTarget::Tab { tab_id })) => {
                self.open_rename(RenameTarget::Tab(tab_id))
            }
            (A::Close, Some(MenuTarget::Tab { tab_id })) => self.request_tab_close(&tab_id).await,
            (A::RenamePane, Some(MenuTarget::Pane { pane_id, .. })) => {
                self.open_rename(RenameTarget::Pane(pane_id))
            }
            (A::ClearPaneName, Some(MenuTarget::Pane { pane_id, .. })) => {
                self.run("pane.rename", json!({"id":pane_id,"name":""}))
                    .await;
            }
            (
                A::SwapWithFocusedPane,
                Some(MenuTarget::Pane {
                    pane_id,
                    source_pane_id: Some(source),
                    ..
                }),
            ) => {
                self.run("pane.swap", json!({"id":source,"other":pane_id}))
                    .await;
            }
            (A::SplitRight | A::SplitDown, Some(MenuTarget::Pane { pane_id, .. })) => {
                self.focus(&pane_id);
                let axis = if action == A::SplitRight {
                    "row"
                } else {
                    "column"
                };
                self.run("pane.split", json!({"id":pane_id,"axis":axis}))
                    .await;
            }
            (A::Zoom, Some(MenuTarget::Pane { pane_id, .. })) => {
                self.focus(&pane_id);
                self.toggle_zoom();
            }
            (A::MovePane, Some(MenuTarget::Pane { pane_id, .. })) => self.open_pane_move(pane_id),
            (A::ParkTerminal, Some(MenuTarget::Pane { pane_id, .. })) => {
                self.run("terminal.park", json!({"id":pane_id})).await;
            }
            (A::ClosePane, Some(MenuTarget::Pane { pane_id, .. })) => {
                self.request_pane_close(&pane_id).await
            }
            (A::KeepTerminal, Some(MenuTarget::Terminal { address })) => {
                let space = self.snapshot.focused_workspace_id.clone();
                let local = herdr_client_lib::commands::local_address(&address, &self.machine)
                    .map(str::to_owned)
                    .unwrap_or(address);
                if self
                    .run(
                        "terminal.keep",
                        json!({"terminal":local,"space":space,"name":""}),
                    )
                    .await
                    .is_some()
                {
                    self.peek = None;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn toggle_group(&mut self, key: String) {
        if !self.chrome.collapsed_groups.remove(&key) {
            self.chrome.collapsed_groups.insert(key);
        }
        self.save_chrome();
    }

    // ---- modes --------------------------------------------------------------

    async fn navigate_key(&mut self, key: KeyEvent) -> Result<bool> {
        let entries = chrome::workspace_entries(&self.snapshot, &self.chrome.collapsed_groups);
        let ids: Vec<String> = entries
            .iter()
            .map(|e| self.snapshot.workspaces[e.index].workspace_id.clone())
            .collect();
        let exit = |this: &mut Self| {
            this.mode = this.copy_or_terminal();
            this.chrome.selected_workspace_id = None;
        };
        if key.code == KeyCode::Esc || self.keys.is_prefix(&key) {
            exit(self);
            return Ok(false);
        }
        let current = self
            .chrome
            .selected_workspace_id
            .as_ref()
            .and_then(|id| ids.iter().position(|w| w == id));
        let navigation = self.keys.navigation(&key).map(str::to_owned);
        match navigation.as_deref() {
            Some("navigate_workspace_up" | "navigate_workspace_down") if !ids.is_empty() => {
                let up = navigation.as_deref() == Some("navigate_workspace_up");
                let next = match (current, up) {
                    (Some(i), true) => i.saturating_sub(1),
                    (Some(i), false) => (i + 1).min(ids.len() - 1),
                    (None, _) => 0,
                };
                self.chrome.selected_workspace_id = Some(ids[next].clone());
                self.chrome.reveal_focused_workspace = true;
                return Ok(false);
            }
            Some("navigate_pane_left") => return self.action("focus_pane_left").await,
            Some("navigate_pane_right") => return self.action("focus_pane_right").await,
            Some("navigate_pane_down") => return self.action("focus_pane_down").await,
            Some("navigate_pane_up") => return self.action("focus_pane_up").await,
            _ => {}
        }
        match key.code {
            KeyCode::Up | KeyCode::Down if !ids.is_empty() => {
                let next = match (current, key.code == KeyCode::Up) {
                    (Some(i), true) => i.saturating_sub(1),
                    (Some(i), false) => (i + 1).min(ids.len() - 1),
                    (None, _) => 0,
                };
                self.chrome.selected_workspace_id = Some(ids[next].clone());
                self.chrome.reveal_focused_workspace = true;
                return Ok(false);
            }
            KeyCode::Enter => {
                if let Some(id) = self.chrome.selected_workspace_id.clone() {
                    self.focus_container(&id);
                }
                exit(self);
                return Ok(false);
            }
            KeyCode::Char(c @ '1'..='9') if key.modifiers.is_empty() => {
                if let Some(id) = ids.get(usize::from(c as u8 - b'1')).cloned() {
                    exit(self);
                    self.focus_container(&id);
                }
                return Ok(false);
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.cycle_pane(key.code == KeyCode::BackTab);
                return Ok(false);
            }
            KeyCode::Left => return self.action("focus_pane_left").await,
            KeyCode::Right => return self.action("focus_pane_right").await,
            _ => {}
        }
        if let Some(action) = self.keys.prefixed(&key) {
            exit(self);
            return self.action(&action).await;
        }
        Ok(false)
    }

    async fn resize_key(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter)
            || self.keys.prefixed(&key).as_deref() == Some("resize_mode")
        {
            self.mode = self.copy_or_terminal();
            return;
        }
        let (vertical, delta) = match key.code {
            KeyCode::Char('h') | KeyCode::Left => (false, -RESIZE_STEP),
            KeyCode::Char('l') | KeyCode::Right => (false, RESIZE_STEP),
            KeyCode::Char('k') | KeyCode::Up => (true, -RESIZE_STEP),
            KeyCode::Char('j') | KeyCode::Down => (true, RESIZE_STEP),
            _ => return,
        };
        self.resize_focused(vertical, delta).await;
    }

    /// Moves the focused pane's divider along one axis, as resize mode does.
    async fn resize_focused(&mut self, vertical: bool, delta: i32) {
        let Some(pane) = self
            .active_id()
            .and_then(|id| self.panes.iter().find(|p| p.id == id))
            .map(|p| p.area)
        else {
            return;
        };
        let view = self.view();
        let Some(doc) = self.document() else {
            return;
        };
        let Some(root) = view
            .active_tab
            .as_deref()
            .and_then(|id| doc.node(id))
            .and_then(|t| t.get("root"))
        else {
            return;
        };
        let surface = self.layout.pane_surface;
        let splitters = layout::splitters(root, surface, view.focused.as_deref());
        let edge = |s: &layout::Splitter| {
            if vertical {
                (s.area.y == pane.bottom().saturating_sub(1) || s.area.y + 1 == pane.y)
                    && s.area.x < pane.right()
                    && s.area.right() > pane.x
            } else {
                (s.area.x == pane.right().saturating_sub(1) || s.area.x + 1 == pane.x)
                    && s.area.y < pane.bottom()
                    && s.area.bottom() > pane.y
            }
        };
        if let Some(splitter) = splitters
            .into_iter()
            .filter(|s| s.vertical == vertical)
            .find(edge)
        {
            let share = splitter.resized_share(delta);
            self.run(
                "pane.resize",
                json!({"id":splitter.id,"next":splitter.next,"share":share}),
            )
            .await;
        }
    }

    fn enter_copy_mode(&mut self) {
        if let Some(id) = self.active_id()
            && let Some(surface) = self.surfaces.get_mut(&id)
        {
            let mut copy = Copy::new(surface.vt.frame());
            if let Some(cursor) = copy.frame.cursor {
                copy.cursor = cursor;
            }
            surface.copy = Some(copy);
            self.mode = Mode::Copy;
        }
    }

    fn copy_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.copy_search = None,
            KeyCode::Enter => {
                let query = self
                    .copy_search
                    .take()
                    .map(|s| s.query.as_str().to_owned())
                    .unwrap_or_default();
                if let Some(id) = self.active_id()
                    && let Some(surface) = self.surfaces.get_mut(&id)
                {
                    match crate::copy::search_history(&mut surface.vt, &query) {
                        Some(found) => surface.copy = Some(found),
                        None => {
                            if let Some(copy) = &mut surface.copy {
                                copy.query = query;
                                copy.selecting = false;
                            }
                            self.toast("no match in retained history", true);
                        }
                    }
                }
            }
            _ => {
                if let Some(search) = &mut self.copy_search {
                    search.query.handle_key(&key);
                }
            }
        }
    }

    fn copy_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(id) = self.active_id() else {
            self.mode = Mode::Terminal;
            return Ok(());
        };
        let Some(surface) = self.surfaces.get_mut(&id) else {
            self.mode = Mode::Terminal;
            return Ok(());
        };
        let Some(copy) = &mut surface.copy else {
            self.mode = Mode::Terminal;
            return Ok(());
        };
        let max_y = copy.frame.rows.len().saturating_sub(1) as u16;
        let max_x = copy
            .frame
            .rows
            .first()
            .map_or(0, |r| r.len().saturating_sub(1)) as u16;
        let row_text = |copy: &Copy| -> Vec<bool> {
            copy.frame
                .rows
                .get(copy.cursor.1 as usize)
                .map(|row| row.iter().map(|c| !c.text.trim().is_empty()).collect())
                .unwrap_or_default()
        };
        match key.code {
            KeyCode::Esc if copy.selecting || !copy.query.is_empty() => {
                copy.selecting = false;
                copy.query.clear();
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                surface.copy = None;
                self.mode = Mode::Terminal;
            }
            KeyCode::Left | KeyCode::Char('h') => copy.cursor.0 = copy.cursor.0.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => copy.cursor.0 = (copy.cursor.0 + 1).min(max_x),
            KeyCode::Up | KeyCode::Char('k') => copy.cursor.1 = copy.cursor.1.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => copy.cursor.1 = (copy.cursor.1 + 1).min(max_y),
            KeyCode::Home | KeyCode::Char('0') => copy.cursor.0 = 0,
            KeyCode::End | KeyCode::Char('$') => copy.cursor.0 = max_x,
            KeyCode::Char('w') | KeyCode::Char('e') => {
                let filled = row_text(copy);
                let mut x = copy.cursor.0 as usize;
                while x < filled.len() && filled[x] {
                    x += 1;
                }
                while x < filled.len() && !filled[x] {
                    x += 1;
                }
                copy.cursor.0 = (x.min(max_x as usize)) as u16;
            }
            KeyCode::Char('b') => {
                let filled = row_text(copy);
                let mut x = copy.cursor.0 as usize;
                x = x.saturating_sub(1);
                while x > 0 && !filled.get(x).copied().unwrap_or(false) {
                    x -= 1;
                }
                while x > 0 && filled.get(x - 1).copied().unwrap_or(false) {
                    x -= 1;
                }
                copy.cursor.0 = x as u16;
            }
            KeyCode::Char('g') => copy.cursor = (0, 0),
            KeyCode::Char('G') => copy.cursor.1 = max_y,
            KeyCode::Char(' ') | KeyCode::Char('v') => {
                copy.anchor = copy.cursor;
                copy.selecting = !copy.selecting;
            }
            KeyCode::Char('y') | KeyCode::Enter => {
                if copy.selecting {
                    clipboard(&copy.text())?;
                    surface.copy = None;
                    self.mode = Mode::Terminal;
                    self.toast("copied to clipboard", false);
                }
            }
            KeyCode::Char('/') => {
                self.copy_search = Some(CopySearch {
                    query: TextEditor::default(),
                });
            }
            KeyCode::Char('n') => {
                let query = copy.query.clone();
                surface.vt.scroll_viewport(1);
                if let Some(found) = crate::copy::search_history(&mut surface.vt, &query) {
                    surface.copy = Some(found);
                }
            }
            KeyCode::PageUp | KeyCode::PageDown => {
                let delta = if key.code == KeyCode::PageUp {
                    -(surface.vt.rows as isize)
                } else {
                    surface.vt.rows as isize
                };
                surface.vt.scroll_viewport(delta);
                surface.copy = Some(Copy::new(surface.vt.frame()));
            }
            _ => {}
        }
        Ok(())
    }

    // ---- mouse --------------------------------------------------------------

    async fn mouse(&mut self, mouse: MouseEvent) -> Result<bool> {
        let (x, y) = (mouse.column, mouse.row);
        if self.overlay.is_some() {
            return self.overlay_mouse(mouse).await;
        }
        if let Some(drag) = self.drag.take() {
            return self.continue_drag(drag, mouse).await;
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.mode == Mode::Prefix {
                    self.mode = Mode::Terminal;
                }
                let hits = &self.hits;
                if contains(hits.sidebar_toggle, x, y) {
                    self.toggle_sidebar();
                } else if contains(hits.new_workspace, x, y) {
                    return self.action("new_workspace").await;
                } else if contains(hits.global_launcher, x, y) {
                    self.overlay = Some(Overlay::GlobalMenu { highlighted: 0 });
                } else if contains(hits.agent_sort_toggle, x, y) {
                    self.config.sort = match self.config.sort {
                        shell::Sort::Spaces => shell::Sort::Priority,
                        shell::Sort::Priority => shell::Sort::Spaces,
                    };
                    self.save_chrome();
                } else if contains(hits.sidebar_section_divider, x, y) {
                    self.drag = Some(Drag::Section);
                } else if contains(hits.sidebar_divider, x, y) && !self.sidebar_collapsed {
                    self.drag = Some(Drag::Sidebar);
                } else if let Some(hit) = hits
                    .workspaces
                    .iter()
                    .find(|h| contains(h.rect, x, y))
                    .cloned()
                {
                    if let Some((_, key)) = hit.group_toggle.filter(|(r, _)| contains(*r, x, y)) {
                        self.toggle_group(key);
                    } else {
                        self.drag = Some(Drag::Press {
                            target: Pressed::Workspace(hit.workspace_id),
                            x,
                            y,
                        });
                    }
                } else if let Some((_, id)) = hits
                    .agents
                    .iter()
                    .find(|(r, _)| contains(*r, x, y))
                    .cloned()
                {
                    self.focus_agent_hit(&id);
                } else if let Some((_, id)) =
                    hits.tabs.iter().find(|(r, _)| contains(*r, x, y)).cloned()
                {
                    self.drag = Some(Drag::Press {
                        target: Pressed::Tab(id),
                        x,
                        y,
                    });
                } else if contains(hits.new_tab, x, y) {
                    return self.action("new_tab").await;
                } else if contains(hits.tab_scroll_left, x, y) {
                    self.chrome.tab_scroll = self.chrome.tab_scroll.saturating_sub(1);
                } else if contains(hits.tab_scroll_right, x, y) {
                    self.chrome.tab_scroll += 1;
                } else if let Some(splitter) = self.splitter_at(x, y) {
                    self.drag = Some(Drag::Splitter(ResizeDrag {
                        origin: if splitter.vertical { y } else { x },
                        value: splitter.resized_share(0),
                        splitter,
                    }));
                } else {
                    self.pane_mouse(mouse)?;
                }
            }
            MouseEventKind::Down(MouseButton::Right)
                if !mouse.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                let hits = &self.hits;
                if let Some(hit) = hits
                    .workspaces
                    .iter()
                    .find(|h| contains(h.rect, x, y))
                    .cloned()
                {
                    self.open_workspace_menu(&hit.workspace_id, x, y);
                } else if let Some((_, id)) =
                    hits.tabs.iter().find(|(r, _)| contains(*r, x, y)).cloned()
                {
                    self.overlay = Some(Overlay::ContextMenu(ContextMenu {
                        target: MenuTarget::Tab { tab_id: id },
                        x,
                        y,
                        highlighted: 0,
                    }));
                } else if let Some((_, id)) = hits
                    .agents
                    .iter()
                    .find(|(r, _)| contains(*r, x, y))
                    .cloned()
                {
                    if let Some(address) = id.strip_prefix('@') {
                        self.overlay = Some(Overlay::ContextMenu(ContextMenu {
                            target: MenuTarget::Terminal {
                                address: address.into(),
                            },
                            x,
                            y,
                            highlighted: 0,
                        }));
                    }
                } else if let Some(pane) = self
                    .panes
                    .iter()
                    .find(|p| contains(p.area, x, y) && !p.id.starts_with('@'))
                    .map(|p| p.id.clone())
                {
                    let source = self.view().focused.filter(|focused| *focused != pane);
                    self.overlay = Some(Overlay::ContextMenu(ContextMenu {
                        target: MenuTarget::Pane {
                            has_manual_label: self
                                .document()
                                .and_then(|d| {
                                    d.node(&pane)
                                        .map(|n| n["name"].as_str().is_some_and(|s| !s.is_empty()))
                                })
                                .unwrap_or(false),
                            pane_id: pane,
                            source_pane_id: source,
                        },
                        x,
                        y,
                        highlighted: 0,
                    }));
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = mouse.kind == MouseEventKind::ScrollUp;
                let hits = &self.hits;
                if contains(hits.workspace_body, x, y) {
                    self.chrome.workspace_scroll = if up {
                        self.chrome.workspace_scroll.saturating_sub(1)
                    } else {
                        self.chrome.workspace_scroll + 1
                    };
                } else if contains(hits.agent_body, x, y) {
                    self.chrome.agent_scroll = if up {
                        self.chrome.agent_scroll.saturating_sub(1)
                    } else {
                        self.chrome.agent_scroll + 1
                    };
                } else if contains(self.layout.tab_bar, x, y) {
                    self.chrome.tab_scroll = if up {
                        self.chrome.tab_scroll.saturating_sub(1)
                    } else {
                        self.chrome.tab_scroll + 1
                    };
                } else if mouse.modifiers.contains(KeyModifiers::ALT) {
                    self.cycle_pane(up);
                } else {
                    self.pane_mouse(mouse)?;
                }
            }
            _ => self.pane_mouse(mouse)?,
        }
        Ok(false)
    }

    /// Linked worktrees move with their main checkout, not on their own.
    fn workspace_draggable(&self, space: &str) -> bool {
        self.snapshot
            .workspace(space)
            .is_some_and(|w| !w.worktree.as_ref().is_some_and(|t| t.is_linked_worktree))
    }

    /// Herdr's drop slots: above each top-level row, and below the last one.
    fn workspace_drop_target_at(&self, y: u16) -> Option<(Option<String>, u16)> {
        let hits = &self.hits;
        if hits.workspace_body.height == 0
            || y < hits.workspace_body.y.saturating_sub(1)
            || y >= hits.new_workspace.y
        {
            return None;
        }
        let snapshot = &self.snapshot;
        let mut slots: Vec<(Option<String>, u16)> = hits
            .workspaces
            .iter()
            .filter(|hit| !hit.indented)
            .filter(|hit| {
                hit.group_toggle.as_ref().is_none_or(|(_, key)| {
                    snapshot
                        .workspaces
                        .iter()
                        .find(|w| {
                            w.worktree
                                .as_ref()
                                .is_some_and(|t| t.key == *key && !t.is_linked_worktree)
                        })
                        .is_some_and(|w| w.workspace_id == hit.workspace_id)
                })
            })
            .map(|hit| (Some(hit.workspace_id.clone()), hit.rect.y.saturating_sub(1)))
            .collect();
        let entries = chrome::workspace_entries(snapshot, &self.chrome.collapsed_groups);
        let last_hit = hits.workspaces.last()?;
        let last_position = entries
            .iter()
            .position(|e| snapshot.workspaces[e.index].workspace_id == last_hit.workspace_id)?;
        let next = entries.get(last_position + 1);
        let next_in_group = next.is_some_and(|entry| {
            entry.indented
                || last_hit.group_toggle.as_ref().is_some_and(|(_, key)| {
                    snapshot.workspaces[entry.index]
                        .worktree
                        .as_ref()
                        .is_some_and(|t| t.key == *key)
                })
        });
        if !next_in_group {
            let before = next.map(|e| snapshot.workspaces[e.index].workspace_id.clone());
            let row = last_hit.rect.bottom();
            if row < hits.new_workspace.y {
                slots.push((before, row));
            }
        }
        slots
            .into_iter()
            .enumerate()
            .min_by_key(|(index, (_, row))| (y.abs_diff(*row), *index))
            .map(|(_, slot)| slot)
    }

    /// The gap between tabs under the pointer: 0 is before the first tab.
    fn tab_drop_index_at(&self, x: u16, y: u16) -> Option<usize> {
        let space = self.snapshot.focused_workspace_id.as_deref()?;
        let tabs: Vec<&str> = self
            .snapshot
            .tabs
            .iter()
            .filter(|t| t.workspace_id == space)
            .map(|t| t.tab_id.as_str())
            .collect();
        let hits = &self.hits;
        let visible: Vec<(usize, Rect)> = hits
            .tabs
            .iter()
            .filter_map(|(rect, id)| tabs.iter().position(|t| t == id).map(|i| (i, *rect)))
            .collect();
        let (first_index, first_rect) = *visible.first()?;
        let (last_index, last_rect) = *visible.last()?;
        if y != first_rect.y {
            return None;
        }
        if contains(hits.tab_scroll_left, x, y) {
            return Some(0);
        }
        if contains(hits.tab_scroll_right, x, y) {
            return Some(tabs.len());
        }
        let left_edge = if first_index == 0 {
            first_rect.x
        } else {
            hits.tab_scroll_left.right()
        };
        let right_edge = if last_index + 1 >= tabs.len() {
            last_rect.right()
        } else {
            hits.tab_scroll_right.x.saturating_sub(1)
        };
        if x <= left_edge {
            return Some(first_index);
        }
        if x >= right_edge {
            return Some(last_index + 1);
        }
        for (index, rect) in visible {
            if x < rect.x + rect.width / 2 {
                return Some(index);
            }
            if x < rect.right() {
                return Some(index + 1);
            }
        }
        Some(last_index + 1)
    }

    async fn move_workspace(&mut self, source: &str, before: Option<&str>) {
        let Some(workspace) = self.snapshot.workspace(source) else {
            return;
        };
        let mut ids = vec![source.to_owned()];
        if let Some(worktree) = &workspace.worktree {
            ids.extend(
                self.snapshot
                    .workspaces
                    .iter()
                    .filter(|w| w.workspace_id != source)
                    .filter(|w| w.worktree.as_ref().is_some_and(|t| t.key == worktree.key))
                    .map(|w| w.workspace_id.clone()),
            );
        }
        if before.is_some_and(|target| ids.iter().any(|id| id == target)) {
            return;
        }
        self.run("space.move", json!({"ids":ids,"before":before}))
            .await;
    }

    /// Moves a tab into a gap between tabs, as Herdr's tab drag does.
    async fn move_tab_to(&mut self, tab: &str, insert: usize) {
        let Some(space) = self.snapshot.tab(tab).map(|t| t.workspace_id.clone()) else {
            return;
        };
        let tabs: Vec<&str> = self
            .snapshot
            .tabs
            .iter()
            .filter(|t| t.workspace_id == space)
            .map(|t| t.tab_id.as_str())
            .collect();
        let Some(current) = tabs.iter().position(|t| *t == tab) else {
            return;
        };
        let index = if insert > current { insert - 1 } else { insert }.min(tabs.len() - 1);
        if index != current {
            self.run("tab.move", json!({"id":tab,"index":index})).await;
        }
    }

    fn focus_agent_hit(&mut self, id: &str) {
        match id.strip_prefix('@') {
            Some(address) => self.focus_agent(address),
            None => self.focus(id),
        }
    }

    fn open_workspace_menu(&mut self, space: &str, x: u16, y: u16) {
        let Some(workspace) = self.snapshot.workspace(space) else {
            return;
        };
        let worktree = workspace.worktree.as_ref();
        let has_worktree_children = worktree.is_some_and(|w| {
            !w.is_linked_worktree
                && self.snapshot.workspaces.iter().any(|c| {
                    c.worktree
                        .as_ref()
                        .is_some_and(|c| c.key == w.key && c.is_linked_worktree)
                })
        });
        self.overlay = Some(Overlay::ContextMenu(ContextMenu {
            target: MenuTarget::Workspace {
                workspace_id: space.into(),
                is_git: worktree.is_some() || workspace.branch.is_some(),
                is_linked_worktree: worktree.is_some_and(|w| w.is_linked_worktree),
                has_worktree_children,
                close_group: chrome::workspace_close_is_group(&self.snapshot, workspace),
                collapsed: worktree.is_some_and(|w| self.chrome.collapsed_groups.contains(&w.key)),
            },
            x,
            y,
            highlighted: 0,
        }));
    }

    fn splitter_at(&self, x: u16, y: u16) -> Option<layout::Splitter> {
        if self.peek.is_some() || self.view().zoomed.is_some() {
            return None;
        }
        let doc = self.document()?;
        let view = self.view();
        let root = view
            .active_tab
            .as_deref()
            .and_then(|id| doc.node(id))
            .and_then(|t| t.get("root"))?;
        layout::splitters(root, self.layout.pane_surface, view.focused.as_deref())
            .into_iter()
            .find(|s| {
                // With frames, the divider is the shared edge: the left pane's right border
                // or the upper pane's bottom border, and the neighbor's first column or row.
                let area = s.area;
                let grown = if s.vertical {
                    Rect::new(area.x, area.y, area.width, 2)
                } else {
                    Rect::new(area.x, area.y, 2, area.height)
                };
                contains(grown, x, y) && !self.panes.iter().any(|p| contains(p.content, x, y))
            })
    }

    async fn continue_drag(&mut self, drag: Drag, mouse: MouseEvent) -> Result<bool> {
        let (x, y) = (mouse.column, mouse.row);
        let released = matches!(mouse.kind, MouseEventKind::Up(_));
        match drag {
            Drag::Press {
                target,
                x: from_x,
                y: from_y,
            } => {
                let moved = x.abs_diff(from_x).max(y.abs_diff(from_y)) >= 1;
                if released {
                    match target {
                        Pressed::Workspace(space) => {
                            if self.mode == Mode::Navigate {
                                self.mode = Mode::Terminal;
                                self.chrome.selected_workspace_id = None;
                            }
                            self.focus_container(&space);
                        }
                        Pressed::Tab(tab) => self.focus_container(&tab),
                    }
                } else if moved && matches!(mouse.kind, MouseEventKind::Drag(_)) {
                    self.drag = Some(match target {
                        Pressed::Workspace(source) if self.workspace_draggable(&source) => {
                            match self.workspace_drop_target_at(y) {
                                Some(target) => Drag::Workspace {
                                    source,
                                    target: Some(target),
                                },
                                None => Drag::Press {
                                    target: Pressed::Workspace(source),
                                    x: from_x,
                                    y: from_y,
                                },
                            }
                        }
                        Pressed::Tab(tab) => match self.tab_drop_index_at(x, y) {
                            Some(insert) => Drag::Tab {
                                tab,
                                insert: Some(insert),
                            },
                            None => Drag::Press {
                                target: Pressed::Tab(tab),
                                x: from_x,
                                y: from_y,
                            },
                        },
                        target => Drag::Press {
                            target,
                            x: from_x,
                            y: from_y,
                        },
                    });
                } else {
                    self.drag = Some(Drag::Press {
                        target,
                        x: from_x,
                        y: from_y,
                    });
                }
            }
            Drag::Workspace { source, .. } => {
                let target = self.workspace_drop_target_at(y);
                if released {
                    if let Some((before, _)) = target {
                        self.move_workspace(&source, before.as_deref()).await;
                    }
                } else {
                    self.drag = Some(Drag::Workspace { source, target });
                }
            }
            Drag::Tab { tab, .. } => {
                let insert = self.tab_drop_index_at(x, y);
                if released {
                    if let Some(insert) = insert {
                        self.move_tab_to(&tab, insert).await;
                    }
                } else {
                    self.drag = Some(Drag::Tab { tab, insert });
                }
            }
            Drag::Sidebar => {
                self.sidebar_width = (x + 1).clamp(
                    self.preferences.ui.sidebar_min_width,
                    self.preferences
                        .ui
                        .sidebar_max_width
                        .max(self.preferences.ui.sidebar_min_width),
                );
                if released {
                    self.save_chrome();
                } else {
                    self.drag = Some(Drag::Sidebar);
                }
            }
            Drag::Section => {
                let height = self.layout.sidebar.height.max(1);
                self.chrome.section_split = (f32::from(y) / f32::from(height)).clamp(0.1, 0.9);
                if released {
                    self.save_chrome();
                } else {
                    self.drag = Some(Drag::Section);
                }
            }
            Drag::Splitter(mut resize) => {
                let position = if resize.splitter.vertical { y } else { x };
                resize.value = resize
                    .splitter
                    .resized_share(i32::from(position) - i32::from(resize.origin));
                if released {
                    self.run(
                        "pane.resize",
                        json!({"id":resize.splitter.id,"next":resize.splitter.next,"share":resize.value}),
                    )
                    .await;
                } else {
                    self.drag = Some(Drag::Splitter(resize));
                }
            }
            Drag::Select(pane) => {
                let Some(content) = self.panes.iter().find(|p| p.id == pane).map(|p| p.content)
                else {
                    return Ok(false);
                };
                if let Some(surface) = self.surfaces.get_mut(&pane)
                    && let Some(copy) = &mut surface.copy
                {
                    copy.cursor = (
                        x.clamp(content.x, content.right().saturating_sub(1)) - content.x,
                        y.clamp(content.y, content.bottom().saturating_sub(1)) - content.y,
                    );
                    if released {
                        let text = copy.text();
                        let moved = copy.cursor != copy.anchor;
                        surface.copy = None;
                        if moved && !text.is_empty() && self.preferences.ui.copy_on_select {
                            clipboard(&text)?;
                            self.toast("copied to clipboard", false);
                        }
                    }
                }
                if !released {
                    self.drag = Some(Drag::Select(pane));
                }
            }
        }
        Ok(false)
    }

    fn pane_mouse(&mut self, mut mouse: MouseEvent) -> Result<()> {
        let (x, y) = (mouse.column, mouse.row);
        let Some(pane) = self
            .panes
            .iter()
            .find(|p| contains(p.area, x, y))
            .map(|p| (p.id.clone(), p.content))
        else {
            return Ok(());
        };
        let (id, content) = pane;
        if matches!(mouse.kind, MouseEventKind::Down(_)) && !id.starts_with('@') {
            self.focus(&id);
        }
        if !contains(content, x, y) {
            return Ok(());
        }
        mouse.column -= content.x;
        mouse.row -= content.y;
        let scroll_lines = self.preferences.ui.scroll_lines();
        let Some(surface) = self.surfaces.get_mut(&id) else {
            return Ok(());
        };
        let left_down = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left));
        if mouse.modifiers.contains(KeyModifiers::CONTROL) && left_down {
            if let Some(link) = crate::copy::link(&mut surface.vt, mouse.column, mouse.row) {
                crate::copy::open_link(&link)?;
            }
            return Ok(());
        }
        let program_mouse =
            surface.vt.mouse_tracking() && !mouse.modifiers.contains(KeyModifiers::SHIFT);
        if left_down && (!program_mouse || self.mode == Mode::Copy) {
            let mut copy = Copy::new(surface.vt.frame());
            copy.anchor = (mouse.column, mouse.row);
            copy.cursor = copy.anchor;
            copy.selecting = true;
            surface.copy = Some(copy);
            self.drag = Some(Drag::Select(id));
            return Ok(());
        }
        let bytes = input::mouse(&mut surface.vt, mouse, scroll_lines);
        if !bytes.is_empty()
            && let Some(stream) = &surface.stream
        {
            stream.try_input(bytes)?;
        }
        Ok(())
    }

    async fn overlay_mouse(&mut self, mouse: MouseEvent) -> Result<bool> {
        let (x, y) = (mouse.column, mouse.row);
        let render = &self.overlay_render;
        let inside = contains(render.area, x, y);
        let scroll = match mouse.kind {
            MouseEventKind::ScrollUp => -1,
            MouseEventKind::ScrollDown => 1,
            _ => 0,
        };
        let left = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left));
        let right = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Right));
        let menu_row = render
            .menu_rows
            .iter()
            .find(|(r, _)| contains(*r, x, y))
            .map(|(_, i)| *i);
        let primary = contains(render.primary, x, y);
        let cancel = contains(render.cancel, x, y);
        let clear = contains(render.clear, x, y);
        let nav_row = render
            .navigator_rows
            .iter()
            .find(|(r, _)| contains(*r, x, y))
            .map(|(_, t)| t.clone());
        let nav_search = contains(render.navigator_search, x, y);
        let worktree_row = render
            .worktree_rows
            .iter()
            .find(|(r, _)| contains(*r, x, y))
            .map(|(_, i)| *i);
        let worktree_search = contains(render.worktree_search, x, y);
        let new_branch = contains(render.worktree_new_branch, x, y);
        let existing_branch = contains(render.worktree_existing_branch, x, y);
        let branch_input = contains(render.worktree_branch_input, x, y);
        let path_input = contains(render.worktree_path_input, x, y);
        let pane_destination = render
            .pane_move_rows
            .iter()
            .find(|(rect, _)| contains(*rect, x, y))
            .map(|(_, id)| id.clone());
        let max_help = render.help_max_scroll;
        let settings_tab = render
            .settings_tabs
            .iter()
            .find(|(r, _)| contains(*r, x, y))
            .map(|(_, s)| *s);
        let settings_choice = render
            .settings_choices
            .iter()
            .find(|(r, _)| contains(*r, x, y))
            .map(|(_, i)| *i);
        let Some(overlay) = self.overlay.take() else {
            return Ok(false);
        };
        if mouse.kind == MouseEventKind::Moved {
            self.overlay = Some(match overlay {
                Overlay::GlobalMenu { highlighted } => Overlay::GlobalMenu {
                    highlighted: menu_row.unwrap_or(highlighted),
                },
                Overlay::ContextMenu(mut menu) => {
                    menu.highlighted = menu_row.unwrap_or(menu.highlighted);
                    Overlay::ContextMenu(menu)
                }
                other => other,
            });
            return Ok(false);
        }
        match overlay {
            Overlay::GlobalMenu { highlighted } => {
                if let (true, Some(index)) = (left, menu_row) {
                    return self.menu_action(global_menu_items()[index].1, None).await;
                }
                if !(left || right) || inside {
                    self.overlay = Some(Overlay::GlobalMenu { highlighted });
                }
            }
            Overlay::ContextMenu(menu) => {
                if let (true, Some(index)) = (left, menu_row)
                    && let Some((_, action)) = menu.items().get(index)
                {
                    return self.menu_action(*action, Some(menu.target)).await;
                }
                if !(left || right) || inside {
                    self.overlay = Some(Overlay::ContextMenu(menu));
                }
            }
            Overlay::Rename(mut rename) => {
                if left && primary {
                    self.submit_rename(rename).await;
                } else if left && cancel {
                } else {
                    if left && clear {
                        rename.input.clear();
                    }
                    self.overlay = Some(Overlay::Rename(rename));
                }
            }
            Overlay::Confirm(confirm) => {
                if left && primary {
                    self.accept_confirm(confirm).await;
                } else if !(left && cancel) {
                    self.overlay = Some(Overlay::Confirm(confirm));
                }
            }
            Overlay::PaneMove(mut picker) => {
                if left && primary {
                    self.submit_pane_move(picker).await;
                } else if !(left && cancel) {
                    if left && let Some(destination) = pane_destination {
                        picker.destination = destination;
                        picker.error = None;
                    }
                    if scroll != 0 {
                        self.step_pane_destination(&mut picker, scroll);
                    }
                    self.overlay = Some(Overlay::PaneMove(picker));
                }
            }
            Overlay::Settings(mut settings) => {
                if left && cancel {
                    self.cancel_settings(settings);
                } else if left && primary {
                    self.apply_settings(settings);
                } else {
                    if left && let Some(section) = settings_tab {
                        self.select_settings_section(&mut settings, section);
                    } else if left && let Some(index) = settings_choice {
                        settings.selected = index;
                        self.preview_settings(&settings);
                    } else if scroll != 0 {
                        self.move_settings_selection(&mut settings, scroll);
                    }
                    self.overlay = Some(Overlay::Settings(settings));
                }
            }
            Overlay::Help(mut help) => {
                if left && cancel {
                    return Ok(false);
                }
                if scroll != 0 {
                    help.scroll =
                        (help.scroll as isize + scroll * 3).clamp(0, max_help as isize) as usize;
                }
                self.overlay = Some(Overlay::Help(help));
            }
            Overlay::Navigator(mut navigator) => {
                if left && let Some(target) = nav_row {
                    self.open_nav_target(target).await;
                    return Ok(false);
                }
                if left && nav_search {
                    navigator.search_focused = true;
                } else if left && !inside {
                    return Ok(false);
                }
                if scroll != 0 {
                    navigator.scroll = (navigator.scroll as isize + scroll * 3).max(0) as usize;
                }
                self.overlay = Some(Overlay::Navigator(navigator));
            }
            Overlay::WorktreeCreate(mut create) => {
                if left && primary && !create.creating {
                    self.submit_worktree_create(create).await;
                } else if !(left && cancel) {
                    if left && !create.creating && (new_branch || existing_branch) {
                        create.new_branch = new_branch;
                        create.error = None;
                    }
                    if left && !create.creating && (branch_input || path_input) {
                        create.path_focused = path_input;
                    }
                    self.overlay = Some(Overlay::WorktreeCreate(create));
                }
            }
            Overlay::WorktreeOpen(mut open) => {
                if left && primary {
                    self.submit_worktree_open(open).await;
                } else if left && cancel {
                } else {
                    if left && let Some(index) = worktree_row {
                        if open.selected == index {
                            self.submit_worktree_open(open).await;
                            return Ok(false);
                        }
                        open.selected = index;
                    }
                    if left && worktree_search {
                        open.search_focused = true;
                    }
                    self.overlay = Some(Overlay::WorktreeOpen(open));
                }
            }
            Overlay::WorktreeRemove(remove) => {
                if left && primary && !remove.removing {
                    self.submit_worktree_remove(remove).await;
                } else if !(left && cancel) {
                    self.overlay = Some(Overlay::WorktreeRemove(remove));
                }
            }
        }
        Ok(false)
    }
}
