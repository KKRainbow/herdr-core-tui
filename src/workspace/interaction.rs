use super::*;
impl Workspace {
    pub(super) async fn host(&mut self, event: Event) -> Result<bool> {
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
                        stream.try_input(bytes)?;
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
                    stream.try_input(bytes)?;
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
                    stream.try_input(surface.vt.paste_bytes(&text))?;
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
                if mouse.modifiers.contains(KeyModifiers::ALT)
                    && matches!(
                        mouse.kind,
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                    )
                {
                    let action = if mouse.kind == MouseEventKind::ScrollUp {
                        "pane.previous"
                    } else {
                        "pane.next"
                    };
                    self.action(action).await?;
                    return Ok(false);
                }
                if let Some(drag) = &mut self.drag {
                    if matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left)) {
                        let position = if drag.splitter.vertical {
                            mouse.row
                        } else {
                            mouse.column
                        };
                        drag.value = drag
                            .splitter
                            .resized_weight(i32::from(position) - i32::from(drag.origin));
                        self.notice = format!("Resize weight {:.2}; release to apply", drag.value);
                        return Ok(false);
                    }
                    if mouse.kind == MouseEventKind::Up(MouseButton::Left) {
                        let drag = self.drag.take().unwrap();
                        self.command(
                            "pane.resize",
                            json!({"id":drag.splitter.id,"weight":drag.value}),
                        )
                        .await?;
                        return Ok(false);
                    }
                }
                if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                    && !self.hits.iter().any(|(rect, hit)| {
                        matches!(hit, Hit::Split(..))
                            && rect.contains((mouse.column, mouse.row).into())
                    })
                    && self.peek.is_none()
                    && self.view().zoomed.is_none()
                    && let Some(doc) = self.document()
                    && let Some(root) = self
                        .view()
                        .active_tab
                        .as_deref()
                        .and_then(|id| doc.node(id))
                        .and_then(|t| t.get("root"))
                    && let Some(area) = self.panes.iter().map(|p| p.area).reduce(|a, b| a.union(b))
                    && let Some(splitter) =
                        layout::splitters(root, area, self.view().focused.as_deref())
                            .into_iter()
                            .find(|s| s.area.contains((mouse.column, mouse.row).into()))
                {
                    self.drag = Some(ResizeDrag {
                        origin: if splitter.vertical {
                            mouse.row
                        } else {
                            mouse.column
                        },
                        value: splitter.weight,
                        splitter,
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
                        Hit::Action(action) => {
                            self.action(action).await?;
                            return Ok(false);
                        }
                        Hit::Split(id, action) => {
                            self.focus(&id);
                            self.action(action).await?;
                            return Ok(false);
                        }
                        Hit::Space(id) | Hit::Tab(id) => {
                            if let Some(doc) = self.document()
                                && let Some(pane) = remembered_pane(&self.view(), &doc, &id)
                            {
                                self.focus(pane);
                            }
                        }
                        Hit::Agent(address) => self.focus_agent(&address),
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
                                stream.try_input(bytes)?;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(false)
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
}
