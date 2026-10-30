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
    pane: String,
    origin: u16,
    width: u16,
    weight: f64,
    value: f64,
}
pub struct Workspace {
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
        self.views.get(&self.machine).cloned().unwrap_or_default()
    }
    fn document(&self) -> Option<herdr_client_lib::document::Document> {
        self.session.document(&self.machine).ok()
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
            let view = self.views.entry(self.machine.clone()).or_default();
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
            self.views
                .entry(self.machine.clone())
                .or_default()
                .focus(&doc, id);
            if let Some(view) = self.views.get_mut(&self.machine)
                && view.zoomed.is_some()
            {
                view.zoomed = view.focused.clone();
            }
            self.peek = None;
        }
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
                terminal: Some(address.clone()),
                description: "Agent".into(),
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
    async fn sync_surfaces(&mut self) -> Result<()> {
        let visible: Vec<_> = self
            .panes
            .iter()
            .filter(|p| p.terminal.is_some())
            .map(|p| p.id.clone())
            .collect();
        for (id, surface) in &mut self.surfaces {
            if !visible.contains(id) {
                surface.stream = None;
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
                surface.stream = None;
                surface.copy = None;
            }
            if surface.stream.is_none() {
                if std::time::Instant::now() < surface.retry_at {
                    continue;
                }
                surface.retry_at = std::time::Instant::now() + Duration::from_secs(2);
                match self.session.client.attach(json!({"terminal":address,"cols":wanted.0,"rows":wanted.1,"resumed":true,"history":1000}),TIME).await {
 Ok((result,stream))=>{let size=size(&result["terminal"],wanted);surface.vt=Vt::new(size.0,size.1);surface.stream=Some(stream);surface.error=None;surface.wanted=wanted;}
 Err(e)=>{surface.error=Some(e.to_string());continue;}
 }
            }
            if surface.wanted != wanted {
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
            } else if command == "tab.focus" {
                layout::first(&node["root"])
            } else {
                node["tabs"]
                    .as_array()
                    .and_then(|t| t.iter().find_map(|t| layout::first(&t["root"])))
            }
            .ok_or("Target has no pane")?;
            self.focus(pane);
            return Ok(());
        }
        if command == "agent.focus" {
            let address = args["terminal"].as_str().ok_or("terminal is required")?;
            if !self
                .session
                .terminals
                .values()
                .any(|r| r["address"] == address)
            {
                return Err("Terminal unavailable".into());
            }
            self.peek = Some(address.into());
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
        let mut view = self.view();
        let result = actions::execute(
            &mut self.session,
            &self.machine,
            &mut view,
            command,
            args.clone(),
        )
        .await?;
        self.views.insert(self.machine.clone(), view);
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
            if matches!(command, "worktree.list" | "worktree.branches" | "help") {
                self.dialog = Some(Dialog::Message(serde_json::to_string_pretty(&result)?));
            } else {
                self.notice = result.to_string();
            }
        }
        Ok(())
    }
    async fn action(&mut self, action: &str) -> Result<()> {
        match action {
            "menu" => self.dialog = Some(Dialog::Menu { selected: 0 }),
            "help" => {
                self.dialog = Some(Dialog::Message(format!(
                    "Ctrl+B then:\ns: space   t: tab   %: horizontal split   \": vertical split\np: next pane   n: next tab   z: zoom   r: rename   x: close\n[: copy mode   :: command   w: new worktree\nCopy: arrows/hjkl move, Space anchors, y copies, / searches, n repeats, Esc exits.\nPageUp/PageDown scroll history. Shift+drag selects.\nClick spaces, tabs and agents to navigate.\nConfig: {}\n\n{}",
                    preferences::config_path().display(),
                    actions::HELP
                )))
            }
            "zoom" => {
                let view = self.views.entry(self.machine.clone()).or_default();
                view.zoomed = if view.zoomed.is_some() {
                    None
                } else {
                    view.focused.clone()
                };
            }
            "agent.next" => {
                let agents: Vec<_> = self
                    .session
                    .terminals
                    .values()
                    .filter(|r| r["reported"] == true || !r["status"].is_null())
                    .filter_map(|r| r["address"].as_str())
                    .collect();
                let current = self.peek.as_deref();
                let index = agents
                    .iter()
                    .position(|a| Some(*a) == current)
                    .map_or(0, |i| (i + 1) % agents.len().max(1));
                self.peek = agents.get(index).map(|s| (*s).into());
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
                        .and_then(|s| s["tabs"].as_array())
                        .and_then(|t| t.iter().find_map(|t| layout::first(&t["root"])))
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
            "pane.next" => {
                let view = self.view();
                let index = self
                    .panes
                    .iter()
                    .position(|p| Some(&p.id) == view.focused.as_ref())
                    .unwrap_or(0);
                if let Some(pane) = self.panes.get((index + 1) % self.panes.len().max(1)) {
                    let id = pane.id.clone();
                    self.focus(&id);
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
                        .and_then(|t| layout::first(&t["root"]))
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
    async fn host(&mut self, event: Event) -> Result<bool> {
        if crate::is_quit(&event) {
            return Ok(true);
        }
        if let Some(mut dialog) = self.dialog.take() {
            match event {
                Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                    KeyCode::Esc => return Ok(false),
                    KeyCode::Char('y') if matches!(dialog, Dialog::Confirm { .. }) => {
                        if let Dialog::Confirm {
                            command, mut args, ..
                        } = dialog
                        {
                            args["confirm"] = json!(true);
                            self.command(&command, args).await?;
                        }
                        return Ok(false);
                    }
                    KeyCode::Up | KeyCode::Down if matches!(dialog, Dialog::Menu { .. }) => {
                        if let Dialog::Menu { selected } = &mut dialog {
                            let n = crate::dialog::MENU.len();
                            *selected = if key.code == KeyCode::Up {
                                (*selected + n - 1) % n
                            } else {
                                (*selected + 1) % n
                            };
                        }
                    }
                    KeyCode::Enter if matches!(dialog, Dialog::Menu { .. }) => {
                        if let Dialog::Menu { selected } = dialog {
                            self.action(crate::dialog::MENU[selected].1).await?;
                        }
                        return Ok(false);
                    }
                    KeyCode::Enter => {
                        if let Dialog::Form(form) = &mut dialog {
                            let args = form.values()?;
                            let command = form.command.clone();
                            if command == "command" {
                                let name = args["command"].as_str().unwrap_or("help");
                                if args.get("arguments").is_none()
                                    || matches!(
                                        name,
                                        "zoom" | "copy" | "help" | "pane.next" | "tab.next"
                                    )
                                {
                                    self.action(name).await?;
                                } else {
                                    self.command(
                                        name,
                                        args["arguments"]
                                            .as_object()
                                            .map(|_| args["arguments"].clone())
                                            .unwrap_or(json!({})),
                                    )
                                    .await?;
                                }
                            } else {
                                self.command(&command, args).await?;
                            }
                            return Ok(false);
                        }
                    }
                    _ => {
                        if let Dialog::Form(form) = &mut dialog {
                            form.handle(key);
                        }
                    }
                },
                Event::Mouse(mouse) => {
                    let (width, height) = crossterm::terminal::size()?;
                    let dialog_height = height.saturating_sub(2).min(match &dialog {
                        Dialog::Form(form) => (form.fields.len() * 3 + 4) as u16,
                        _ => 22,
                    });
                    let top = (height - dialog_height) / 2 + 1;
                    let left = (width - width.saturating_sub(4).min(90)) / 2;
                    if mouse.column > left
                        && mouse.row >= top
                        && mouse.row < top + dialog_height.saturating_sub(2)
                    {
                        match &mut dialog {
                            Dialog::Menu { selected } => {
                                if mouse.kind == MouseEventKind::ScrollDown {
                                    *selected = (*selected + 1).min(crate::dialog::MENU.len() - 1);
                                }
                                if mouse.kind == MouseEventKind::ScrollUp {
                                    *selected = selected.saturating_sub(1);
                                }
                                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                                    let start = selected
                                        .saturating_sub(dialog_height.saturating_sub(3) as usize);
                                    let index = start + (mouse.row - top) as usize;
                                    if let Some((_, action)) = crate::dialog::MENU.get(index) {
                                        self.action(action).await?;
                                        return Ok(false);
                                    }
                                }
                            }
                            Dialog::Form(form)
                                if mouse.kind == MouseEventKind::Down(MouseButton::Left) =>
                            {
                                form.active = ((mouse.row - top) / 3) as usize;
                                form.active = form.active.min(form.fields.len().saturating_sub(1));
                            }
                            _ => {}
                        }
                    }
                }
                Event::Paste(text) => {
                    if let Dialog::Form(form) = &mut dialog
                        && let Some(field) = form.fields.get_mut(form.active)
                    {
                        field.editor.insert(&text);
                    }
                }
                _ => {}
            }
            self.dialog = Some(dialog);
            return Ok(false);
        }
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                if let Some(id) = self.active_id()
                    && let Some(surface) = self.surfaces.get_mut(&id)
                    && let Some(stream) = &surface.stream
                {
                    let bytes = input::key(&mut surface.vt, key);
                    if !bytes.is_empty() {
                        stream.input(bytes).await?;
                    }
                }
                return Ok(false);
            }
            if self.prefix {
                self.prefix = false;
                let name = match key.code {
                    KeyCode::Char(c) => c.to_string(),
                    KeyCode::Left => "h".into(),
                    KeyCode::Right => "l".into(),
                    KeyCode::Up => "k".into(),
                    KeyCode::Down => "j".into(),
                    _ => String::new(),
                };
                if let Some(action) = self.preferences.keys.get(&name).cloned() {
                    self.action(&action).await?;
                }
                return Ok(false);
            }
            if key.code == KeyCode::Char('b') && key.modifiers.contains(KeyModifiers::CONTROL) {
                self.prefix = true;
                return Ok(false);
            }
            if let Some(id) = self.active_id()
                && let Some(surface) = self.surfaces.get_mut(&id)
            {
                if let Some(copy) = &mut surface.copy {
                    let max_y = copy.frame.rows.len().saturating_sub(1) as u16;
                    let max_x = copy
                        .frame
                        .rows
                        .first()
                        .map_or(0, |r| r.len().saturating_sub(1))
                        as u16;
                    match key.code {
                        KeyCode::Esc => surface.copy = None,
                        KeyCode::Left | KeyCode::Char('h') => {
                            copy.cursor.0 = copy.cursor.0.saturating_sub(1)
                        }
                        KeyCode::Right | KeyCode::Char('l') => {
                            copy.cursor.0 = (copy.cursor.0 + 1).min(max_x)
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            copy.cursor.1 = copy.cursor.1.saturating_sub(1)
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            copy.cursor.1 = (copy.cursor.1 + 1).min(max_y)
                        }
                        KeyCode::Home => copy.cursor.0 = 0,
                        KeyCode::End => copy.cursor.0 = max_x,
                        KeyCode::Char(' ') => {
                            copy.anchor = copy.cursor;
                            copy.selecting = true;
                        }
                        KeyCode::Char('y') => {
                            clipboard(&copy.text())?;
                            self.notice = "Selection copied using OSC 52".into();
                            surface.copy = None;
                        }
                        KeyCode::Char('/') => {
                            self.dialog = Some(Dialog::Form(Form::new("search", &json!({}))))
                        }
                        KeyCode::Char('n') => {
                            let query = copy.query.clone();
                            surface.vt.scroll_viewport(1);
                            if let Some(found) =
                                crate::copy::search_history(&mut surface.vt, &query)
                            {
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
                    return Ok(false);
                }
                surface.vt.scroll_to_bottom();
                let bytes = input::key(&mut surface.vt, key);
                if !bytes.is_empty()
                    && let Some(stream) = &surface.stream
                {
                    stream.input(bytes).await?;
                }
            }
            return Ok(false);
        }
        match event {
            Event::Paste(text) => {
                if let Some(id) = self.active_id()
                    && let Some(surface) = self.surfaces.get(&id)
                    && let Some(stream) = &surface.stream
                {
                    stream.input(surface.vt.paste_bytes(&text)).await?;
                }
            }
            Event::FocusLost => {
                self.focused = false;
                for surface in self.surfaces.values() {
                    if let Some(stream) = &surface.stream {
                        stream.focus(false).await?;
                    }
                }
            }
            Event::FocusGained => {
                self.focused = true;
                self.activate().await?;
            }
            Event::Mouse(mut mouse) => {
                if let Some(drag) = &mut self.drag {
                    if matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left)) {
                        let width = (i32::from(drag.width) + i32::from(mouse.column)
                            - i32::from(drag.origin))
                        .max(3) as f64;
                        drag.value = (drag.weight * width / f64::from(drag.width.max(1))).max(0.05);
                        self.notice = format!("Resize weight {:.2}; release to apply", drag.value);
                        return Ok(false);
                    }
                    if mouse.kind == MouseEventKind::Up(MouseButton::Left) {
                        let drag = self.drag.take().unwrap();
                        self.command("pane.resize", json!({"id":drag.pane,"weight":drag.value}))
                            .await?;
                        return Ok(false);
                    }
                }
                if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                    && let Some(pane) = self.panes.iter().find(|p| {
                        mouse.column == p.area.right().saturating_sub(1)
                            && mouse.row > p.area.y
                            && mouse.row < p.area.bottom().saturating_sub(1)
                    })
                {
                    let weight = self
                        .document()
                        .and_then(|d| d.node(&pane.id).and_then(|n| n["size"]["weight"].as_f64()))
                        .unwrap_or(1.0);
                    self.drag = Some(ResizeDrag {
                        pane: pane.id.clone(),
                        origin: mouse.column,
                        width: pane.area.width,
                        weight,
                        value: weight,
                    });
                    return Ok(false);
                }

                if mouse.kind == MouseEventKind::Down(MouseButton::Right)
                    && !mouse.modifiers.contains(KeyModifiers::SHIFT)
                {
                    if let Some(pane) = self
                        .panes
                        .iter()
                        .find(|p| p.area.contains((mouse.column, mouse.row).into()))
                    {
                        let id = pane.id.clone();
                        self.focus(&id);
                    }
                    self.dialog = Some(Dialog::Menu { selected: 0 });
                    return Ok(false);
                }
                if let Some((_, hit)) = self
                    .hits
                    .iter()
                    .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
                    .cloned()
                    && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                {
                    match hit {
                        Hit::Space(id) => {
                            if let Some(doc) = self.document()
                                && let Some(pane) = doc
                                    .node(&id)
                                    .and_then(|s| s["tabs"].as_array())
                                    .and_then(|t| t.iter().find_map(|t| layout::first(&t["root"])))
                            {
                                self.focus(pane);
                            }
                        }
                        Hit::Tab(id) => {
                            if let Some(doc) = self.document()
                                && let Some(pane) =
                                    doc.node(&id).and_then(|t| layout::first(&t["root"]))
                            {
                                self.focus(pane);
                            }
                        }
                        Hit::Agent(address) => self.peek = Some(address),
                        Hit::Pane(id) => {
                            if !id.starts_with('@') {
                                self.focus(&id);
                            }
                        }
                    }
                }
                if let Some(pane) = self
                    .panes
                    .iter()
                    .find(|p| inner(p.area).contains((mouse.column, mouse.row).into()))
                {
                    let area = inner(pane.area);
                    mouse.column -= area.x;
                    mouse.row -= area.y;
                    if let Some(surface) = self.surfaces.get_mut(&pane.id) {
                        if mouse.modifiers.contains(KeyModifiers::CONTROL)
                            && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                        {
                            if let Some(link) =
                                crate::copy::link(&mut surface.vt, mouse.column, mouse.row)
                            {
                                crate::copy::open_link(&link)?;
                            }
                            return Ok(false);
                        }

                        if mouse.modifiers.contains(KeyModifiers::SHIFT) || surface.copy.is_some() {
                            match mouse.kind {
                                MouseEventKind::Down(MouseButton::Left) => {
                                    let mut copy = Copy::new(surface.vt.frame());
                                    copy.anchor = (mouse.column, mouse.row);
                                    copy.cursor = copy.anchor;
                                    copy.selecting = true;
                                    surface.copy = Some(copy);
                                }
                                MouseEventKind::Drag(MouseButton::Left) => {
                                    if let Some(copy) = &mut surface.copy {
                                        copy.cursor = (mouse.column, mouse.row);
                                    }
                                }
                                MouseEventKind::Up(MouseButton::Left) => {
                                    if let Some(copy) = &surface.copy {
                                        clipboard(&copy.text())?;
                                    }
                                }
                                _ => {}
                            }
                        } else {
                            let bytes = input::mouse(&mut surface.vt, mouse);
                            if !bytes.is_empty()
                                && let Some(stream) = &surface.stream
                            {
                                stream.input(bytes).await?;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(false)
    }
    async fn activate(&mut self) -> Result<()> {
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
                    Some("restarted" | "lost") => surface.stream = None,
                    _ => {}
                }
            } else if value["method"] == "missed" {
                surface.stream = None;
            }
        }
        self.reconcile();
    }
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
pub async fn run(config: HcConfig) -> Result<()> {
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
    let session = Session::connect(client, machines, TIME).await?;
    let mut app = Workspace {
        attention: Default::default(),
        drag: None,
        session,
        views: preferences::read_views(&config.session),
        machine: String::new(),
        surfaces: BTreeMap::new(),
        panes: vec![],
        hits: vec![],
        preferences: preferences::load()?,
        dialog: None,
        prefix: false,
        focused: true,
        peek: None,
        notice: String::new(),
    };
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
    let mut connected = true;
    let mut retry = tokio::time::interval(Duration::from_secs(2));
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if dirty {
            let area = screen.size()?;
            app.geometry(Rect::new(0, 0, area.width, area.height));
            if connected && let Err(e) = app.sync_surfaces().await {
                app.notice = e.to_string();
            }
            app.draw(&mut screen)?;
            dirty = false;
        }
        enum Next {
            Host(Option<io::Result<Event>>),
            Output(String, std::result::Result<Option<StreamEvent>, Error>),
            Update(std::result::Result<Value, Error>),
            Retry,
        }
        let next = tokio::select! {
        _=&mut interrupt=>break,
        event=events.next()=>Next::Host(event),
        output=next_output(&mut app.surfaces)=>Next::Output(output.0,output.1),
        update=app.session.next(),if connected=>Next::Update(update),
        _=retry.tick()=>Next::Retry,
        };
        match next {
            Next::Host(event) => {
                let Some(event) = event.transpose()? else {
                    break;
                };
                let old = app.active_id();
                let activate =
                    matches!(event, Event::Mouse(_) | Event::Key(_) | Event::FocusGained);
                match app.host(event).await {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(e) => app.dialog = Some(Dialog::Message(e.to_string())),
                }
                if old != app.active_id() && activate {
                    app.geometry(Rect::new(0, 0, screen.size()?.width, screen.size()?.height));
                    if let Err(e) = app.sync_surfaces().await {
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
            Next::Output(id, _) => {
                if let Some(surface) = app.surfaces.get_mut(&id) {
                    surface.stream = None;
                    surface.error = Some("Terminal stream ended; waiting for core".into());
                }
                dirty = true;
            }
            Next::Update(Ok(value)) => {
                app.notification(&value);
                dirty = true;
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
                if !connected {
                    match config.connect(TIME).await {
                        Ok(client) => match app.session.reconnect(client).await {
                            Ok(()) => {
                                connected = true;
                                app.notice = "Reconnected".into();
                                app.reconcile();
                                dirty = true;
                            }
                            Err(e) => app.notice = e.to_string(),
                        },
                        Err(e) => app.notice = e.to_string(),
                    }
                } else if app.surfaces.values().any(|s| s.stream.is_none()) {
                    dirty = true;
                }
            }
        }
    }
    preferences::save_views(&config.session, &app.views)?;
    Ok(())
}
