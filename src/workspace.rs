mod interaction;
mod render;
use crate::{
    Host, Result, actions,
    copy::{Copy, clipboard},
    input,
    layout::{self, Pane},
    paint, preferences,
    shell::{
        self, Hits, Mode,
        chrome::ChromeState,
        keys::Keymap,
        overlays::{Overlay, OverlayRender},
        snapshot::{self, GitInfo, Snapshot},
    },
    vt::Vt,
};
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use futures_util::{StreamExt, future::select_all};
use herdr_client_lib::{
    client::{Error, Stream, StreamEvent},
    commands::qualify,
    sync::Session,
    transport::HcConfig,
    view::{Identity, View},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{self, stdout},
    time::{Duration, Instant},
};
const TIME: Duration = Duration::from_secs(5);
const GIT_REFRESH: Duration = Duration::from_secs(10);
const TOAST: Duration = Duration::from_secs(4);
struct Surface {
    ended: bool,
    retry_at: Instant,
    address: String,
    stream: Option<Stream>,
    vt: Vt,
    copy: Option<Copy>,
    wanted: (u16, u16),
    error: Option<String>,
}
struct ResizeDrag {
    splitter: layout::Splitter,
    origin: u16,
    value: f64,
}
enum Pressed {
    Workspace(String),
    Tab(String),
}
enum Drag {
    Splitter(ResizeDrag),
    Sidebar,
    Section,
    Select(String),
    /// A press on a space or tab: a click on release, or a reorder once it moves.
    Press {
        target: Pressed,
        x: u16,
        y: u16,
    },
    /// A space (with its worktree group) and where it would land: before a space, or at the end.
    Workspace {
        source: String,
        target: Option<(Option<String>, u16)>,
    },
    Tab {
        tab: String,
        insert: Option<usize>,
    },
}
struct Toast {
    text: String,
    error: bool,
    until: Instant,
}
/// A copy-mode search being typed in the mode bar.
struct CopySearch {
    query: crate::editor::TextEditor,
}
pub struct Workspace {
    view_key: String,
    identities: BTreeMap<String, String>,
    renders: u64,
    parsed_bytes: u64,
    attention: herdr_client_lib::view::Attention,
    sequences: BTreeMap<Identity, u64>,
    words: BTreeMap<Identity, Option<String>>,
    sequence: u64,
    drag: Option<Drag>,
    pub session: Session,
    pub views: BTreeMap<String, View>,
    pub machine: String,
    machines: Vec<String>,
    surfaces: BTreeMap<String, Surface>,
    panes: Vec<Pane>,
    hits: Hits,
    overlay: Option<Overlay>,
    overlay_render: OverlayRender,
    mode: Mode,
    chrome: ChromeState,
    chrome_saved: preferences::Chrome,
    config: shell::Config,
    keys: Keymap,
    preferences: preferences::Preferences,
    sidebar_width: u16,
    sidebar_collapsed: bool,
    layout: shell::Layout,
    /// The host size the pane layout was computed for.
    area: Rect,
    snapshot: Snapshot,
    git: HashMap<String, GitInfo>,
    git_checked: HashMap<String, Instant>,
    focused: bool,
    peek: Option<String>,
    previous_pane: Option<String>,
    copy_search: Option<CopySearch>,
    notice: String,
    toast: Option<Toast>,
}
impl Workspace {
    fn view(&self) -> View {
        self.views.get(&self.view_id()).cloned().unwrap_or_default()
    }
    fn view_id(&self) -> String {
        self.identities
            .get(&self.machine)
            .cloned()
            .unwrap_or_else(|| format!("unavailable:{}", self.machine))
    }
    fn view_mut(&mut self) -> &mut View {
        let id = self.view_id();
        self.views.entry(id).or_default()
    }
    fn toast(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        self.notice = text.clone();
        self.toast = Some(Toast {
            text,
            error,
            until: Instant::now() + TOAST,
        });
    }
    async fn refresh_identities(&mut self) -> Result<bool> {
        let machines = self
            .session
            .client
            .call("machine.list", json!({}), TIME)
            .await?;
        let identities = identities(&self.session.client, &machines);
        let mut changed = self.identities != identities;
        self.identities = identities;
        self.machines = machine_names(&machines);
        changed |= self.session.poll_availability().await?;
        Ok(changed)
    }
    fn document(&self) -> Option<herdr_client_lib::document::Document> {
        self.session.document(&self.machine).ok()
    }
    async fn recover_placements(&mut self) -> bool {
        let outcomes = self.session.reconcile_placements().await;
        if outcomes.is_empty() {
            return false;
        }
        let errors = outcomes
            .iter()
            .filter_map(|outcome| {
                use herdr_client_lib::sync::WriteOutcome;
                let error = match &outcome.result {
                    Ok(WriteOutcome::Written) => return None,
                    Ok(WriteOutcome::Conflict) => "workspace changed during recovery".into(),
                    Ok(WriteOutcome::Unavailable(error)) => error.clone(),
                    Err(error) => error.to_string(),
                };
                Some(format!("{}: {error}", outcome.terminal))
            })
            .collect::<Vec<_>>();
        if !errors.is_empty() {
            self.toast(format!("Recovery failed: {}", errors.join("; ")), true);
        }
        self.reconcile();
        true
    }

    fn reconcile(&mut self) {
        let active = self
            .active_id()
            .and_then(|id| self.surfaces.get(&id).map(|s| s.address.clone()));
        for (identity, record) in &self.session.terminals {
            let word = snapshot::word(record).map(str::to_owned);
            if self.words.get(identity) != Some(&word) {
                self.sequence += 1;
                self.sequences.insert(identity.clone(), self.sequence);
                self.words.insert(identity.clone(), word.clone());
            }
            self.attention.observe(
                identity.clone(),
                word,
                record["address"].as_str() == active.as_deref(),
            );
        }
        if let Some(doc) = self.document() {
            let view = self.view_mut();
            view.reconcile(&doc);
            if view.focused.is_none()
                && let Some(pane) = doc.value()["spaces"].as_array().and_then(|s| {
                    s.iter().find_map(|s| {
                        s["tabs"]
                            .as_array()
                            .and_then(|t| t.iter().find_map(|t| layout::first(&t["root"])))
                    })
                })
            {
                view.focus(&doc, pane);
            }
        }
    }
    fn focus(&mut self, id: &str) {
        if let Some(doc) = self.document() {
            let before = self.view().focused;
            self.view_mut().focus(&doc, id);
            if let Some(view) = self.views.get_mut(&self.view_id())
                && view.zoomed.is_some()
            {
                view.zoomed = view.focused.clone();
            }
            if before.as_deref() != Some(id) && before.is_some() {
                self.previous_pane = before.clone();
            }
            if self.peek.take().is_some() || before.as_deref() != Some(id) {
                self.chrome.reveal_focused_tab = true;
                self.chrome.reveal_focused_workspace = true;
            }
        }
    }
    fn focus_agent(&mut self, address: &str) {
        let placed = self.session.documents.keys().find_map(|machine| {
            let doc = self.session.document(machine).ok()?;
            agent_pane(doc.value(), address, machine).map(|id| (machine.clone(), id))
        });
        if let Some((machine, pane)) = placed {
            self.machine = machine;
            self.focus(&pane);
        } else {
            self.peek = Some(address.into());
        }
    }
    /// Focuses a space or tab at the pane its viewer last used there.
    fn focus_container(&mut self, id: &str) {
        if let Some(doc) = self.document()
            && let Some(pane) = remembered_pane(&self.view(), &doc, id)
        {
            self.focus(pane);
        }
    }
    fn check_pane_action(&self, command: &str) -> Result<()> {
        if self.peek.is_some()
            && (command.starts_with("pane.")
                || matches!(
                    command,
                    "terminal.park" | "split.row" | "split.column" | "zoom"
                ))
        {
            return Err("This terminal has no workspace pane. Keep it in a tab first.".into());
        }
        Ok(())
    }
    fn geometry(&mut self, area: Rect) {
        self.area = area;
        self.layout = shell::layout(
            area.width,
            area.height,
            self.sidebar_width,
            self.sidebar_collapsed,
        );
        let body = self.layout.pane_surface;
        self.panes.clear();
        let view = self.view();
        if let Some(address) = &self.peek {
            self.panes.push(Pane {
                id: format!("@{address}"),
                terminal: Some(address.clone()),
                description: "Terminal".into(),
                area: body,
                content: body,
                bordered: false,
                scrollbar: None,
            });
        } else if let Some(doc) = self.document()
            && let Some(root) = view
                .active_tab
                .as_deref()
                .and_then(|id| doc.node(id))
                .and_then(|t| t.get("root"))
        {
            self.panes = layout::panes(root, body, view.focused.as_deref(), view.zoomed.as_deref());
            let tab_panes = layout::navigation_ids(root).len();
            let bordered = self.panes.len() > 1 || (view.zoomed.is_some() && tab_panes > 1);
            for pane in &mut self.panes {
                pane.bordered = bordered;
            }
        }
        for pane in &mut self.panes {
            let alternate = self
                .surfaces
                .get(&pane.id)
                .is_some_and(|s| s.vt.alt_screen());
            let (content, scrollbar) = shell::panes::terminal_rect(
                shell::panes::inner(pane.area, pane.bordered),
                alternate,
            );
            pane.content = content;
            pane.scrollbar = scrollbar;
        }
    }
    fn refresh_snapshot(&mut self) {
        let doc = self.document();
        let view = self.view();
        self.snapshot = snapshot::build(&snapshot::Sources {
            document: doc.as_ref().map(|d| d.value()),
            view: &view,
            machine: &self.machine,
            terminals: &self.session.terminals,
            attention: &self.attention,
            sequences: &self.sequences,
            git: &self.git,
            peek: self.peek.as_deref(),
        });
    }
    /// Asks git about each space's folder on its machine, at most every few seconds per folder.
    async fn refresh_git(&mut self) -> bool {
        let folders: HashSet<String> = self
            .snapshot
            .workspaces
            .iter()
            .map(|w| w.cwd.clone())
            .filter(|cwd| !cwd.is_empty())
            .collect();
        self.git.retain(|cwd, _| folders.contains(cwd));
        self.git_checked.retain(|cwd, _| folders.contains(cwd));
        let git = herdr_client_lib::worktrees::Git {
            client: self.session.client.clone(),
            machine: self.machine.clone(),
            deadline: TIME,
        };
        let mut changed = false;
        for cwd in folders {
            if self
                .git_checked
                .get(&cwd)
                .is_some_and(|at| at.elapsed() < GIT_REFRESH)
            {
                continue;
            }
            self.git_checked.insert(cwd.clone(), Instant::now());
            let info = git_info(&git, &cwd).await;
            if self.git.get(&cwd) != info.as_ref() {
                changed = true;
                match info {
                    Some(info) => self.git.insert(cwd, info),
                    None => self.git.remove(&cwd),
                };
            }
        }
        changed
    }
    fn invalidate_git(&mut self) {
        self.git_checked.clear();
    }
    async fn sync_surfaces(&mut self, activate: bool) -> Result<()> {
        let visible: Vec<_> = self
            .panes
            .iter()
            .filter(|p| p.terminal.is_some())
            .map(|p| p.id.clone())
            .collect();
        for (id, surface) in &mut self.surfaces {
            if !visible.contains(id) {
                surface.stream = None;
                surface.retry_at = Instant::now();
            }
        }
        if self.surfaces.len() > 32 {
            self.surfaces.retain(|id, _| visible.contains(id));
        }
        for pane in &self.panes {
            let Some(local) = &pane.terminal else {
                continue;
            };
            let area = pane.content;
            if area.is_empty() {
                continue;
            }
            let address = if pane.id.starts_with('@') {
                local.clone()
            } else {
                qualify(local, &self.machine)
            };
            let wanted = (area.width.max(1), area.height.max(1));
            let surface = self
                .surfaces
                .entry(pane.id.clone())
                .or_insert_with(|| Surface {
                    ended: false,
                    retry_at: Instant::now(),
                    address: address.clone(),
                    stream: None,
                    vt: Vt::new(wanted.0, wanted.1),
                    copy: None,
                    wanted,
                    error: None,
                });
            if surface.address != address {
                surface.address = address.clone();
                surface.ended = false;
                surface.stream = None;
                surface.copy = None;
            }
            if surface.stream.is_none() {
                if surface.ended || Instant::now() < surface.retry_at {
                    continue;
                }
                surface.retry_at = Instant::now() + Duration::from_secs(2);
                let finished =
                    self.session.terminals.values().any(|record| {
                        record["address"] == address && record["state"] == "finished"
                    });
                if finished {
                    match self
                        .session
                        .client
                        .call("terminal.read", json!({"terminal":address}), TIME)
                        .await
                    {
                        Ok(result) => {
                            surface.vt = Vt::new(wanted.0, wanted.1);
                            surface.vt.write(
                                result["text"]
                                    .as_str()
                                    .unwrap_or("")
                                    .replace('\n', "\r\n")
                                    .as_bytes(),
                            );
                            surface.ended = true;
                            surface.error = None;
                        }
                        Err(error) => surface.error = Some(error.to_string()),
                    }
                    continue;
                }
                match self.session.client.attach(json!({"terminal":address,"cols":wanted.0,"rows":wanted.1,"resumed":true,"history":1000}),TIME).await {
                    Ok((result, stream)) => {
                        let size = size(&result["terminal"],wanted);
                        surface.vt = Vt::new(size.0,size.1);
                        surface.stream = Some(stream);
                        surface.error = None;
                        surface.wanted = wanted;
                    }
                    Err(error) => {surface.error = Some(error.to_string());continue;}
                }
            }
            if surface.wanted != wanted || activate {
                surface.wanted = wanted;
                if self.focused
                    && let Some(stream) = &surface.stream
                {
                    stream.resize(wanted.0, wanted.1).await?;
                }
            }
        }
        Ok(())
    }
    fn active_id(&self) -> Option<String> {
        self.peek
            .as_ref()
            .map(|a| format!("@{a}"))
            .or(self.view().focused)
    }
    /// Runs a shared or private command and reports failures as a toast.
    async fn run(&mut self, command: &str, args: Value) -> Option<Value> {
        match self.command(command, args).await {
            Ok(value) => Some(value),
            Err(error) => {
                self.toast(error.to_string(), true);
                None
            }
        }
    }
    async fn command(&mut self, command: &str, args: Value) -> Result<Value> {
        if matches!(command, "pane.focus" | "tab.focus" | "space.focus") {
            let id = args["id"].as_str().ok_or("id is required")?;
            let doc = self.document().ok_or("Document unavailable")?;
            let node = doc.node(id).ok_or("Target unavailable")?;
            let pane = if command == "pane.focus" {
                Some(id)
            } else {
                remembered_pane(&self.view(), &doc, id).or_else(|| layout::first(&node["root"]))
            }
            .ok_or("Target has no pane")?;
            self.focus(pane);
            return Ok(json!({"ok":true}));
        }
        if matches!(command, "agent.focus" | "terminal.focus") {
            let address = args["terminal"].as_str().ok_or("terminal is required")?;
            if !self
                .session
                .terminals
                .values()
                .any(|r| r["address"] == address)
            {
                return Err("Terminal unavailable".into());
            }
            self.focus_agent(address);
            return Ok(json!({"ok":true}));
        }
        if command == "machine" {
            self.machine = args["machine"].as_str().unwrap_or("").into();
            if !self.session.documents.contains_key(&self.machine) {
                self.session.refresh(&self.machine).await;
            }
            self.peek = None;
            self.invalidate_git();
            self.reconcile();
            return Ok(json!({"ok":true}));
        }
        self.check_pane_action(command)?;
        let mut view = self.view();
        let result =
            actions::execute(&mut self.session, &self.machine, &mut view, command, args).await?;
        self.views.insert(self.view_id(), view);
        self.reconcile();
        self.chrome.reveal_focused_tab = true;
        self.chrome.reveal_focused_workspace = true;
        if result["ok"] == false {
            let message = result["error"]
                .as_str()
                .or_else(|| result["workspace"]["error"].as_str())
                .or_else(|| result["failure"]["error"].as_str())
                .or_else(|| result["git"]["stderr"].as_str().filter(|s| !s.is_empty()))
                .map_or_else(|| format!("{command} did not complete"), str::to_owned);
            return Err(message.trim().to_owned().into());
        }
        Ok(result)
    }
    fn notification(&mut self, value: &Value) {
        let event = &value["params"];
        let address = event["terminal"].as_str();
        for surface in self.surfaces.values_mut() {
            if address == Some(&surface.address) {
                match event["type"].as_str() {
                    Some("size") => {
                        let size = size(&event["data"], surface.wanted);
                        surface.vt.resize(size.0, size.1);
                    }
                    Some("restarted" | "lost") => {
                        surface.stream = None;
                        surface.ended = false;
                    }
                    Some("finished") => surface.ended = true,
                    _ => {}
                }
            } else if value["method"] == "missed" {
                surface.stream = None;
            }
        }
        if matches!(event["type"].as_str(), Some("turn")) {
            self.invalidate_git();
        }
        self.reconcile();
    }
    fn save_chrome(&mut self) {
        self.chrome_saved.sidebar_width = Some(self.sidebar_width);
        self.chrome_saved.sidebar_collapsed = self.sidebar_collapsed;
        self.chrome_saved.sidebar_section_split = self.chrome.section_split;
        self.chrome_saved.agent_panel_sort = Some(
            match self.config.sort {
                shell::Sort::Spaces => "spaces",
                shell::Sort::Priority => "priority",
            }
            .into(),
        );
        self.chrome_saved.collapsed_groups = self.chrome.collapsed_groups.iter().cloned().collect();
        if let Err(error) = preferences::save_chrome(&self.view_key, &self.chrome_saved) {
            self.toast(error.to_string(), true);
        }
    }
    fn reload_config(&mut self) -> Result<()> {
        self.preferences = preferences::load()?;
        let sort = self.config.sort;
        self.config = self.preferences.shell();
        if self.chrome_saved.agent_panel_sort.is_some() {
            self.config.sort = sort;
        }
        self.keys = Keymap::new(&self.preferences.keys);
        Ok(())
    }
}

/// Branch, checkout, and worktree identity for one folder, read with argv-based git.
async fn git_info(git: &herdr_client_lib::worktrees::Git, cwd: &str) -> Option<GitInfo> {
    let output = git
        .run(
            cwd,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--show-toplevel",
                "--git-common-dir",
                "--absolute-git-dir",
                "--abbrev-ref",
                "HEAD",
            ],
        )
        .await
        .ok()?;
    if output.code != Some(0) {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = text.lines().collect();
    let [top, common, dir, head] = lines.as_slice() else {
        return None;
    };
    let mut info = GitInfo {
        top: (*top).into(),
        branch: (*head != "HEAD").then(|| (*head).to_owned()),
        common_dir: common.trim_end_matches('/').into(),
        linked: dir.trim_end_matches('/') != common.trim_end_matches('/'),
        ahead_behind: None,
    };
    if info.branch.is_some()
        && let Ok(counts) = git
            .run(
                cwd,
                &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
            )
            .await
        && counts.code == Some(0)
    {
        let text = String::from_utf8_lossy(&counts.stdout);
        let mut numbers = text.split_whitespace().filter_map(|n| n.parse().ok());
        if let (Some(ahead), Some(behind)) = (numbers.next(), numbers.next()) {
            info.ahead_behind = Some((ahead, behind));
        }
    }
    Some(info)
}

fn remembered_pane<'a>(
    view: &View,
    doc: &'a herdr_client_lib::document::Document,
    target: &str,
) -> Option<&'a str> {
    for id in view.focused.iter().chain(view.history.iter().rev()) {
        if doc
            .location(id)
            .is_some_and(|(space, tab)| space == target || tab == target)
        {
            return doc.node(id).and_then(|n| n["id"].as_str());
        }
    }
    let node = doc.node(target)?;
    layout::first(&node["root"]).or_else(|| {
        node["tabs"]
            .as_array()?
            .iter()
            .find_map(|t| layout::first(&t["root"]))
    })
}

fn agent_pane(node: &Value, address: &str, machine: &str) -> Option<String> {
    if node["type"] == "block"
        && node["content"]["terminal"]
            .as_str()
            .is_some_and(|a| qualify(a, machine) == address)
    {
        return node["id"].as_str().map(str::to_owned);
    }
    for key in ["spaces", "tabs", "children"] {
        if let Some(found) = node[key]
            .as_array()
            .and_then(|items| items.iter().find_map(|n| agent_pane(n, address, machine)))
        {
            return Some(found);
        }
    }
    node.get("root")
        .and_then(|n| agent_pane(n, address, machine))
}

fn identities(
    client: &herdr_client_lib::client::Client,
    machines: &Value,
) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    if let Some(id) = client.capabilities()["core"]["id"].as_str() {
        result.insert(String::new(), id.into());
    }
    for record in machines["machines"].as_array().into_iter().flatten() {
        if let (Some(name), Some(id)) = (record["name"].as_str(), record["id"].as_str()) {
            result.insert(name.into(), id.into());
        }
    }
    result
}
fn machine_names(machines: &Value) -> Vec<String> {
    let mut names = vec![String::new()];
    names.extend(
        machines["machines"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["name"].as_str().map(str::to_owned)),
    );
    names
}
fn size(record: &Value, fallback: (u16, u16)) -> (u16, u16) {
    (
        record["cols"]
            .as_u64()
            .map(|n| n.clamp(1, 65535) as u16)
            .unwrap_or(fallback.0),
        record["rows"]
            .as_u64()
            .map(|n| n.clamp(1, 65535) as u16)
            .unwrap_or(fallback.1),
    )
}
async fn next_output(
    surfaces: &mut BTreeMap<String, Surface>,
) -> (String, std::result::Result<Option<StreamEvent>, Error>) {
    let futures: Vec<_> = surfaces
        .iter_mut()
        .filter_map(|(id, s)| {
            s.stream
                .as_mut()
                .map(|stream| Box::pin(async move { (id.clone(), stream.next().await) }))
        })
        .collect();
    if futures.is_empty() {
        std::future::pending().await
    } else {
        select_all(futures).await.0
    }
}
pub async fn run(
    config: HcConfig,
    initial_machine: String,
    instance: Option<String>,
) -> Result<()> {
    let instance = instance.unwrap_or_else(|| format!("tui-{}", std::process::id()));
    let mut endpoint = crate::control::Endpoint::bind(&config.session, &instance)?;
    let view_key = format!("{}:{instance}", config.session);
    let client = config.connect(TIME).await?;
    let machine_list = client.call("machine.list", json!({}), TIME).await?;
    let machines = machine_names(&machine_list);
    let identities = identities(&client, &machine_list);
    let session = Session::connect(client, machines.clone(), TIME).await?;
    let preferences = preferences::load()?;
    let chrome_saved = preferences::read_chrome(&view_key);
    let mut shell_config = preferences.shell();
    if let Some(sort) = &chrome_saved.agent_panel_sort {
        shell_config.sort = if sort == "priority" {
            shell::Sort::Priority
        } else {
            shell::Sort::Spaces
        };
    }
    let mut app = Workspace {
        view_key: view_key.clone(),
        identities,
        renders: 0,
        parsed_bytes: 0,
        attention: Default::default(),
        sequences: BTreeMap::new(),
        words: BTreeMap::new(),
        sequence: 0,
        drag: None,
        session,
        views: preferences::read_views(&view_key),
        machine: initial_machine,
        machines,
        surfaces: BTreeMap::new(),
        panes: vec![],
        hits: Hits::default(),
        overlay: None,
        overlay_render: OverlayRender::default(),
        mode: Mode::Terminal,
        chrome: ChromeState {
            section_split: chrome_saved.sidebar_section_split,
            collapsed_groups: chrome_saved.collapsed_groups.iter().cloned().collect(),
            reveal_focused_tab: true,
            reveal_focused_workspace: true,
            ..ChromeState::default()
        },
        sidebar_width: chrome_saved
            .sidebar_width
            .unwrap_or(preferences.sidebar_width),
        sidebar_collapsed: chrome_saved.sidebar_collapsed,
        chrome_saved,
        config: shell_config,
        keys: Keymap::new(&preferences.keys),
        preferences,
        layout: shell::Layout::default(),
        area: Rect::default(),
        snapshot: Snapshot::default(),
        git: HashMap::new(),
        git_checked: HashMap::new(),
        focused: true,
        peek: None,
        previous_pane: None,
        copy_search: None,
        notice: format!("Instance {instance}"),
        toast: None,
    };
    if !app.session.documents.contains_key(&app.machine) {
        app.session.refresh(&app.machine).await;
    }
    app.recover_placements().await;
    app.reconcile();
    app.refresh_snapshot();
    app.refresh_git().await;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        crate::restore();
        previous(info);
    }));
    let _host = Host::enter()?;
    let mut screen = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut events = EventStream::new();
    let mut interrupt = Box::pin(tokio::signal::ctrl_c());
    let mut dirty = true;
    let mut activate_layout = true;
    let mut render_due = true;
    let mut redraw = tokio::time::interval(Duration::from_millis(16));
    redraw.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut connected = true;
    let mut retry = tokio::time::interval(Duration::from_secs(2));
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if dirty && render_due {
            let area = screen.size()?;
            app.geometry(Rect::new(0, 0, area.width, area.height));
            if connected && let Err(e) = app.sync_surfaces(activate_layout).await {
                app.toast(e.to_string(), true);
            }
            activate_layout = false;
            app.refresh_snapshot();
            // The host can resize while streams resize; redraw at the size the frame used.
            let resized = app.draw(&mut screen)?;
            dirty = resized;
            activate_layout |= resized;
            render_due = false;
        }
        enum Next {
            Host(Option<io::Result<Event>>),
            Output(String, std::result::Result<Option<StreamEvent>, Error>),
            Update(std::result::Result<Value, Error>),
            Retry,
            Paint,
            Control(crate::control::Request),
        }
        let next = tokio::select! {
        _=&mut interrupt=>break,
        event=events.next()=>Next::Host(event),
        Some(request)=endpoint.requests.recv()=>Next::Control(request),
        output=next_output(&mut app.surfaces)=>Next::Output(output.0,output.1),
        update=app.session.next(),if connected=>Next::Update(update),
        _=retry.tick()=>Next::Retry,
        _=redraw.tick(),if dirty=>Next::Paint,
        };
        match next {
            Next::Paint => render_due = true,
            Next::Control(request) => {
                let quit = request.command == "quit";
                let result = if request.command == "state" {
                    Ok(
                        json!({"instance":instance,"session":config.session,"machine":app.machine,"view":app.view(),"connected":connected,"notice":app.notice,"document_version":app.session.documents.get(&app.machine).map(|s|s.version),"document_available":app.document().is_some(),"metrics":{"renders":app.renders,"parsed_bytes":app.parsed_bytes,"attached":app.surfaces.values().filter(|s|s.stream.is_some()).count(),"cached":app.surfaces.len()},"panes":app.panes.iter().map(|p|json!({"id":p.id,"terminal":p.terminal})).collect::<Vec<_>>()}),
                    )
                } else if quit {
                    Ok(json!({"ok":true}))
                } else {
                    let result = match request.command.as_str() {
                        "zoom" => {
                            app.toggle_zoom();
                            Ok(())
                        }
                        "pane.focus" | "tab.focus" | "space.focus" | "agent.focus"
                        | "terminal.focus" | "machine" => app
                            .command(&request.command, request.args)
                            .await
                            .map(|_| ()),
                        _ => Err(
                            "Only private view commands are accepted; use do for shared operations"
                                .into(),
                        ),
                    };
                    result.map(|()| json!({"ok":true,"view":app.view()}))
                };
                let _ = request.reply.send(match result {
                    Ok(value) => value,
                    Err(e) => json!({"error":e.to_string()}),
                });
                dirty |= request.command != "state";
                activate_layout |= request.command != "state";
                if quit {
                    let _ = tokio::time::timeout(Duration::from_secs(2), request.delivered).await;
                    break;
                }
            }
            Next::Host(event) => {
                let Some(event) = event.transpose()? else {
                    break;
                };
                let old = app.active_id();
                activate_layout |= matches!(event, Event::FocusGained | Event::Resize(_, _));
                let activate =
                    matches!(event, Event::Mouse(_) | Event::Key(_) | Event::FocusGained);
                match app.host(event).await {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(e) => app.toast(e.to_string(), true),
                }
                if old != app.active_id() && activate {
                    let size = screen.size()?;
                    app.geometry(Rect::new(0, 0, size.width, size.height));
                    if let Err(e) = app.sync_surfaces(true).await {
                        app.toast(e.to_string(), true);
                    }
                    if let Err(e) = app.activate().await {
                        app.toast(e.to_string(), true);
                    }
                }
                dirty = true;
            }
            Next::Output(id, Ok(Some(StreamEvent::Data(bytes)))) => {
                if let Some(surface) = app.surfaces.get_mut(&id) {
                    app.parsed_bytes += bytes.len() as u64;
                    let alternate = surface.vt.alt_screen();
                    surface.vt.write(&bytes);
                    activate_layout |= alternate != surface.vt.alt_screen();
                    if let Some(stream) = &surface.stream
                        && let Err(e) = stream.consumed(bytes.len() as u32).await
                    {
                        surface.error = Some(e.to_string());
                        surface.stream = None;
                    }
                }
                dirty = true;
            }
            Next::Output(id, outcome) => {
                if let Some(surface) = app.surfaces.get_mut(&id) {
                    surface.stream = None;
                    surface.ended |= matches!(outcome, Ok(Some(StreamEvent::Closed(ref value))) if value["reason"] == "exited");
                    surface.error = Some(
                        if surface.ended {
                            "Terminal finished"
                        } else {
                            "Terminal stream ended; waiting for core"
                        }
                        .into(),
                    );
                }
                dirty = true;
            }
            Next::Update(Ok(value)) => {
                if value["method"] == "missed"
                    && let Err(e) = app.refresh_identities().await
                {
                    app.toast(e.to_string(), true);
                }
                if value["method"] != "heartbeat" {
                    app.notification(&value);
                    dirty = true;
                }
            }
            Next::Update(Err(e)) => {
                app.toast(format!("Disconnected: {e}; reconnecting"), true);
                connected = false;
                for surface in app.surfaces.values_mut() {
                    surface.stream = None;
                }
                dirty = true;
            }
            Next::Retry => {
                if app
                    .toast
                    .as_ref()
                    .is_some_and(|t| Instant::now() >= t.until)
                {
                    app.toast = None;
                    dirty = true;
                }
                if connected {
                    dirty |= app.recover_placements().await;
                    app.refresh_snapshot();
                    dirty |= app.refresh_git().await;
                }
                if connected && app.session.documents.keys().any(|name| !name.is_empty()) {
                    match app.refresh_identities().await {
                        Ok(changed) => {
                            dirty |= changed;
                            if changed {
                                app.reconcile();
                            }
                        }
                        Err(error) => app.toast(error.to_string(), true),
                    }
                }
                if !connected {
                    match config.connect(TIME).await {
                        Ok(client) => match app.session.reconnect(client).await {
                            Ok(()) => {
                                connected = true;
                                if let Err(e) = app.refresh_identities().await {
                                    app.toast(e.to_string(), true);
                                }
                                app.toast("Reconnected", false);
                                app.reconcile();
                                dirty = true;
                            }
                            Err(e) => app.toast(e.to_string(), true),
                        },
                        Err(e) => app.toast(e.to_string(), true),
                    }
                } else if app.panes.iter().any(|p| {
                    app.surfaces
                        .get(&p.id)
                        .is_some_and(|s| s.stream.is_none() && !s.ended)
                }) {
                    dirty = true;
                }
            }
        }
    }
    preferences::save_views(&view_key, &app.views)?;
    app.save_chrome();
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_restores_tab_and_space_focus() {
        use herdr_client_lib::document::{Document, terminal_pane};
        let mut doc = Document::default();
        for space in ["s1", "s2"] {
            doc.create_space(space, "", "").unwrap();
        }
        for (space, tab, pane) in [("s1", "t1", "a"), ("s1", "t2", "b"), ("s2", "t3", "c")] {
            doc.create_tab(space, tab, "", terminal_pane(pane, pane))
                .unwrap();
        }
        doc.split(
            "b",
            "split",
            herdr_client_lib::document::Axis::Row,
            terminal_pane("d", "d"),
        )
        .unwrap();
        let mut view = View::default();
        for id in ["a", "d", "c"] {
            view.focus(&doc, id);
        }
        assert_eq!(remembered_pane(&view, &doc, "s1"), Some("d"));
        assert_eq!(remembered_pane(&view, &doc, "t2"), Some("d"));
        doc.remove("d").unwrap();
        assert_eq!(remembered_pane(&view, &doc, "t2"), Some("b"));
    }
    #[test]
    fn agent_target_resolves_only_placed_terminal() {
        let doc = json!({"spaces":[{"tabs":[{"root":{"type":"row","children":[
            {"type":"block","id":"p","content":{"terminal":"t1"}}
        ]}}]}]});
        assert_eq!(
            agent_pane(&doc, &qualify("t1", "remote"), "remote"),
            Some("p".into())
        );
        assert_eq!(agent_pane(&doc, "missing", "remote"), None);
    }
}
