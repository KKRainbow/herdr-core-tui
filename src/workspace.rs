mod interaction;
mod render;
use crate::{
    Host, Result, actions,
    copy::{Copy, clipboard},
    dialog::{Dialog, Form},
    input,
    layout::{self, Pane},
    paint, preferences,
    vt::Vt,
};
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use futures_util::{StreamExt, future::select_all};
use herdr_client_lib::{
    client::{Error, Stream, StreamEvent},
    commands::qualify,
    sync::Session,
    transport::HcConfig,
    view::View,
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{self, stdout},
    time::Duration,
};
const TIME: Duration = Duration::from_secs(5);
struct Surface {
    ended: bool,
    retry_at: std::time::Instant,
    address: String,
    stream: Option<Stream>,
    vt: Vt,
    copy: Option<Copy>,
    wanted: (u16, u16),
    error: Option<String>,
}
#[derive(Clone)]
enum Hit {
    Space(String),
    Tab(String),
    Pane(String),
    Agent(String),
}
struct ResizeDrag {
    splitter: layout::Splitter,
    origin: u16,
    value: f64,
}
pub struct Workspace {
    session_name: String,
    identities: BTreeMap<String, String>,
    renders: u64,
    parsed_bytes: u64,
    attention: herdr_client_lib::view::Attention,
    drag: Option<ResizeDrag>,
    pub session: Session,
    pub views: BTreeMap<String, View>,
    pub machine: String,
    surfaces: BTreeMap<String, Surface>,
    panes: Vec<Pane>,
    hits: Vec<(Rect, Hit)>,
    preferences: preferences::Preferences,
    dialog: Option<Dialog>,
    prefix: bool,
    focused: bool,
    peek: Option<String>,
    notice: String,
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
    async fn refresh_identities(&mut self) -> Result<bool> {
        let machines = self
            .session
            .client
            .call("machine.list", json!({}), TIME)
            .await?;
        let identities = identities(&self.session.client, &machines);
        let mut changed = self.identities != identities;
        self.identities = identities;
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
            self.notice = format!("Recovery failed: {}", errors.join("; "));
        }
        self.reconcile();
        true
    }

    fn reconcile(&mut self) {
        let active = self
            .active_id()
            .and_then(|id| self.surfaces.get(&id).map(|s| s.address.clone()));
        for (identity, record) in &self.session.terminals {
            let word = record["status"]["state"]
                .as_str()
                .or_else(|| record["status"].as_str())
                .map(str::to_owned);
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
            self.view_mut().focus(&doc, id);
            if let Some(view) = self.views.get_mut(&self.view_id())
                && view.zoomed.is_some()
            {
                view.zoomed = view.focused.clone();
            }
            self.peek = None;
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
        self.panes.clear();
        let sidebar = self.preferences.sidebar_width.min(area.width / 3);
        let body = Rect::new(
            area.x + sidebar,
            area.y + 1,
            area.width - sidebar,
            area.height.saturating_sub(2),
        );
        let view = self.view();
        if let Some(address) = &self.peek {
            self.panes.push(Pane {
                id: format!("@{address}"),
                name: None,
                terminal: Some(address.clone()),
                description: "Terminal".into(),
                area: body,
            });
        } else if let Some(doc) = self.document()
            && let Some(root) = view
                .active_tab
                .as_deref()
                .and_then(|id| doc.node(id))
                .and_then(|t| t.get("root"))
        {
            self.panes = layout::panes(root, body, view.focused.as_deref(), view.zoomed.as_deref());
        }
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
                surface.retry_at = std::time::Instant::now();
            }
        }
        if self.surfaces.len() > 32 {
            self.surfaces.retain(|id, _| visible.contains(id));
        }
        for pane in &self.panes {
            let Some(local) = &pane.terminal else {
                continue;
            };
            let area = inner(pane.area);
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
                    retry_at: std::time::Instant::now(),
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
                if surface.ended {
                    continue;
                }
                if std::time::Instant::now() < surface.retry_at {
                    continue;
                }
                surface.retry_at = std::time::Instant::now() + Duration::from_secs(2);
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
    async fn command(&mut self, command: &str, args: Value) -> Result<()> {
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
            return Ok(());
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
            return Ok(());
        }
        if command == "machine" {
            self.machine = args["machine"].as_str().unwrap_or("").into();
            if !self.session.documents.contains_key(&self.machine) {
                self.session.refresh(&self.machine).await;
            }
            self.peek = None;
            self.reconcile();
            return Ok(());
        }
        if command == "search" {
            if let Some(id) = self.active_id()
                && let Some(surface) = self.surfaces.get_mut(&id)
            {
                if let Some(copy) = crate::copy::search_history(
                    &mut surface.vt,
                    args["query"].as_str().unwrap_or(""),
                ) {
                    surface.copy = Some(copy);
                } else {
                    self.notice = "No match in retained history".into();
                }
            }
            return Ok(());
        }
        self.check_pane_action(command)?;
        let mut view = self.view();
        let result = actions::execute(
            &mut self.session,
            &self.machine,
            &mut view,
            command,
            args.clone(),
        )
        .await?;
        self.views.insert(self.view_id(), view);
        self.reconcile();
        if result.get("confirmation").is_some() {
            let mut args = args;
            args["expected"] = result.clone();
            self.dialog = Some(Dialog::Confirm {
                command: command.into(),
                args,
                description: serde_json::to_string_pretty(&result)?,
            });
        } else {
            if result["ok"] == false
                || matches!(command, "worktree.list" | "worktree.branches" | "help")
            {
                self.dialog = Some(Dialog::Message(serde_json::to_string_pretty(&result)?));
            } else {
                self.notice = result.to_string();
            }
        }
        Ok(())
    }
    async fn action(&mut self, action: &str) -> Result<()> {
        self.check_pane_action(action)?;
        match action {
            "menu" => self.dialog = Some(Dialog::Menu { selected: 0 }),
            "help" => {
                self.dialog = Some(Dialog::Message(format!(
                    "Ctrl+B then:\ns: space   t: tab   %: horizontal split   \": vertical split\np: next pane   n: next tab   z: zoom   r: rename   x: close\n[: copy mode   :: command   w: new worktree\nCopy: arrows/hjkl move, Space anchors, y copies, / searches, n repeats, Esc exits.\nPageUp/PageDown scroll history. Shift+drag selects.\nClick spaces, tabs and terminals to navigate. Ctrl+B e cycles live terminals. Alt+wheel cycles all panes, including hidden stack panes.\nConfig: {}\n\n{}",
                    preferences::config_path().display(),
                    actions::HELP
                )))
            }
            "zoom" => {
                let view = self.view_mut();
                view.zoomed = if view.zoomed.is_some() {
                    None
                } else {
                    view.focused.clone()
                };
            }
            "agent.next" | "terminal.next" => {
                let agents: Vec<_> = self
                    .session
                    .terminals
                    .values()
                    .filter(|r| {
                        if action == "terminal.next" {
                            r["state"] == "live"
                        } else {
                            r["reported"] == true || !r["status"].is_null()
                        }
                    })
                    .filter_map(|r| r["address"].as_str())
                    .collect();
                let active = self
                    .active_id()
                    .and_then(|id| self.surfaces.get(&id))
                    .map(|s| s.address.as_str());
                let current = self.peek.as_deref().or(active);
                let index = agents
                    .iter()
                    .position(|a| Some(*a) == current)
                    .map_or(0, |i| (i + 1) % agents.len().max(1));
                if let Some(address) = agents.get(index).map(|s| s.to_string()) {
                    self.focus_agent(&address);
                }
            }
            "space.next" => {
                if let Some(doc) = self.document()
                    && let Some(spaces) = doc.value()["spaces"].as_array()
                {
                    let view = self.view();
                    let index = spaces
                        .iter()
                        .position(|s| s["id"].as_str() == view.active_space.as_deref())
                        .unwrap_or(0);
                    if let Some(id) = spaces
                        .get((index + 1) % spaces.len().max(1))
                        .and_then(|s| s["id"].as_str())
                        .and_then(|id| remembered_pane(&view, &doc, id))
                    {
                        self.focus(id);
                    }
                }
            }
            "pane.left" | "pane.right" | "pane.up" | "pane.down" => {
                if let Some(current) = self
                    .active_id()
                    .and_then(|id| self.panes.iter().find(|p| p.id == id))
                {
                    let center = |p: &Pane| {
                        (
                            i32::from(p.area.x) + i32::from(p.area.width) / 2,
                            i32::from(p.area.y) + i32::from(p.area.height) / 2,
                        )
                    };
                    let (x, y) = center(current);
                    let chosen = self
                        .panes
                        .iter()
                        .filter(|p| p.id != current.id)
                        .filter(|p| {
                            let (px, py) = center(p);
                            match action {
                                "pane.left" => px < x,
                                "pane.right" => px > x,
                                "pane.up" => py < y,
                                _ => py > y,
                            }
                        })
                        .min_by_key(|p| {
                            let (px, py) = center(p);
                            (px - x).abs() + (py - y).abs()
                        })
                        .map(|p| p.id.clone());
                    if let Some(id) = chosen {
                        self.focus(&id);
                    }
                }
            }
            "pane.next" | "pane.previous" => {
                let view = self.view();
                if let Some(doc) = self.document()
                    && let Some(tab) = view.active_tab.as_deref().and_then(|id| doc.node(id))
                {
                    let ids = layout::navigation_ids(&tab["root"]);
                    if !ids.is_empty() {
                        let index = ids
                            .iter()
                            .position(|id| Some(id) == view.focused.as_ref())
                            .unwrap_or(0);
                        let step = if action == "pane.previous" {
                            ids.len() - 1
                        } else {
                            1
                        };
                        self.focus(&ids[(index + step) % ids.len()]);
                    }
                }
            }
            "tab.next" => {
                let view = self.view();
                if let Some(doc) = self.document()
                    && let Some(tabs) = view
                        .active_space
                        .as_deref()
                        .and_then(|id| doc.node(id))
                        .and_then(|s| s["tabs"].as_array())
                {
                    let index = tabs
                        .iter()
                        .position(|t| t["id"].as_str() == view.active_tab.as_deref())
                        .unwrap_or(0);
                    if let Some(id) = tabs
                        .get((index + 1) % tabs.len().max(1))
                        .and_then(|t| t["id"].as_str())
                        .and_then(|id| remembered_pane(&view, &doc, id))
                    {
                        self.focus(id);
                    }
                }
            }
            "copy" => {
                if let Some(id) = self.active_id()
                    && let Some(surface) = self.surfaces.get_mut(&id)
                {
                    surface.copy = Some(Copy::new(surface.vt.frame()));
                }
            }
            "pane.close" | "space.close" | "tab.close" | "terminal.park" => {
                self.command(action, json!({})).await?
            }
            _ => {
                let (command, mut defaults) = match action {
                    "split.row" => ("pane.split", json!({"axis":"row"})),
                    "split.column" => ("pane.split", json!({"axis":"column"})),
                    _ => (action, json!({})),
                };
                let view = self.view();
                if matches!(
                    command,
                    "pane.rename" | "pane.resize" | "pane.move" | "pane.swap" | "pane.split"
                ) {
                    defaults["id"] = json!(view.focused);
                }
                if command == "tab.rename" || command == "tab.move" {
                    defaults["id"] = json!(view.active_tab);
                }
                if command == "space.rename" || command == "space.move" {
                    defaults["id"] = json!(view.active_space);
                }
                if command == "tab.new" {
                    defaults["space"] = json!(view.active_space);
                }
                self.dialog = Some(Dialog::Form(Form::new(command, &defaults)));
            }
        }
        Ok(())
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
        self.reconcile();
    }
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
fn inner(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
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
    let mut machines = vec![String::new()];
    machines.extend(
        machine_list["machines"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["name"].as_str().map(str::to_owned)),
    );
    let identities = identities(&client, &machine_list);
    let session = Session::connect(client, machines, TIME).await?;
    let mut app = Workspace {
        session_name: config.session.clone(),
        identities,
        renders: 0,
        parsed_bytes: 0,
        attention: Default::default(),
        drag: None,
        session,
        views: preferences::read_views(&view_key),
        machine: initial_machine,
        surfaces: BTreeMap::new(),
        panes: vec![],
        hits: vec![],
        preferences: preferences::load()?,
        dialog: None,
        prefix: false,
        focused: true,
        peek: None,
        notice: format!("Instance {instance} · Ctrl+B commands · Ctrl+Q quit"),
    };
    if !app.session.documents.contains_key(&app.machine) {
        app.session.refresh(&app.machine).await;
    }
    app.recover_placements().await;
    app.reconcile();
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
                app.notice = e.to_string();
            }
            activate_layout = false;
            app.draw(&mut screen)?;
            dirty = false;
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
                        "zoom" => app.action("zoom").await,
                        "pane.focus" | "tab.focus" | "space.focus" | "agent.focus"
                        | "terminal.focus" | "machine" => {
                            app.command(&request.command, request.args).await
                        }
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
                    Err(e) => app.dialog = Some(Dialog::Message(e.to_string())),
                }
                if old != app.active_id() && activate {
                    app.geometry(Rect::new(0, 0, screen.size()?.width, screen.size()?.height));
                    if let Err(e) = app.sync_surfaces(true).await {
                        app.notice = e.to_string();
                    }
                    if let Err(e) = app.activate().await {
                        app.notice = e.to_string();
                    }
                }
                dirty = true;
            }
            Next::Output(id, Ok(Some(StreamEvent::Data(bytes)))) => {
                if let Some(surface) = app.surfaces.get_mut(&id) {
                    app.parsed_bytes += bytes.len() as u64;
                    surface.vt.write(&bytes);
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
                    app.notice = e.to_string();
                }
                if value["method"] != "heartbeat" {
                    app.notification(&value);
                    dirty = true;
                }
            }
            Next::Update(Err(e)) => {
                app.notice = format!("Disconnected: {e}; reconnecting");
                connected = false;
                for surface in app.surfaces.values_mut() {
                    surface.stream = None;
                }
                dirty = true;
            }
            Next::Retry => {
                if connected {
                    dirty |= app.recover_placements().await;
                }
                if connected && app.session.documents.keys().any(|name| !name.is_empty()) {
                    match app.refresh_identities().await {
                        Ok(changed) => {
                            dirty |= changed;
                            if changed {
                                app.reconcile();
                            }
                        }
                        Err(error) => app.notice = error.to_string(),
                    }
                }
                if !connected {
                    match config.connect(TIME).await {
                        Ok(client) => match app.session.reconnect(client).await {
                            Ok(()) => {
                                connected = true;
                                if let Err(e) = app.refresh_identities().await {
                                    app.notice = e.to_string();
                                }
                                app.notice = "Reconnected".into();
                                app.reconcile();
                                dirty = true;
                            }
                            Err(e) => app.notice = e.to_string(),
                        },
                        Err(e) => app.notice = e.to_string(),
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
