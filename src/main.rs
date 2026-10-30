mod actions;
mod cli;
mod config_edit;
mod connection;
mod control;
mod copy;
mod editor;
mod input;
mod layout;
mod paint;
mod preferences;
mod scrollback;
mod shell;
mod sound;
mod vt;
mod workspace;
use crossterm::{
    cursor,
    event::{self, Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use herdr_client_lib::client::StreamEvent;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    style::{Color, Modifier, Style},
};
use std::{
    io::{self, stdout},
    time::Duration,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
struct Host;
impl Host {
    fn enter() -> io::Result<Self> {
        let guard = Self;
        Self::modes()?;
        Ok(guard)
    }
    fn modes() -> io::Result<()> {
        terminal::enable_raw_mode()?;
        execute!(
            stdout(),
            EnterAlternateScreen,
            event::EnableBracketedPaste,
            event::EnableFocusChange,
            event::EnableMouseCapture,
            event::PushKeyboardEnhancementFlags(
                event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | event::KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            )
        )
    }
    /// Hands the host terminal to another program, then takes it back.
    pub fn suspend<T>(run: impl FnOnce() -> T) -> io::Result<T> {
        restore();
        let result = run();
        Self::modes()?;
        Ok(result)
    }
}
fn restore() {
    let _ = execute!(
        stdout(),
        event::PopKeyboardEnhancementFlags,
        event::DisableMouseCapture,
        event::DisableFocusChange,
        event::DisableBracketedPaste,
        cursor::Show,
        crossterm::style::ResetColor,
        crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
        LeaveAlternateScreen
    );
    let _ = terminal::disable_raw_mode();
}
impl Drop for Host {
    fn drop(&mut self) {
        restore();
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("herder-tui: {error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", cli::HELP);
        return Ok(());
    }
    if args == ["--version"] {
        println!("herder-tui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let options = cli::parse(args)?;
    if cli::headless(&options).await? {
        return Ok(());
    }
    if options.positional.is_empty() {
        return workspace::run(options.config, options.machine, options.instance).await;
    }
    if options.positional.len() != 1 {
        return Err("Expected one terminal address".into());
    }
    let target = herdr_client_lib::commands::qualify(&options.positional[0], &options.machine);
    let config = options.config;
    let mut size = terminal::size()?;
    let mut connection =
        Some(connection::Connection::connect(&config, &target, size, false).await?);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
    let _host = Host::enter()?;
    let mut screen = Terminal::new(CrosstermBackend::new(stdout()))?;
    let initial_size = connection.as_ref().unwrap().size();
    let mut vt = vt::Vt::new(initial_size.0, initial_size.1);
    let mut events = EventStream::new();
    let mut interrupt = Box::pin(tokio::signal::ctrl_c());
    let mut focused = true;
    let mut dirty = true;
    let mut retry = tokio::time::interval(Duration::from_secs(1));
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if dirty {
            draw(&mut screen, &mut vt)?;
            dirty = false;
        }
        let Some(current) = connection.as_mut() else {
            tokio::select! {
                _=&mut interrupt=>break,
                result=connection::Connection::connect(&config,&target,size,true)=> {
                    match result {
                        Ok(new)=>{let dimensions=new.size();vt=vt::Vt::new(dimensions.0,dimensions.1);connection=Some(new);dirty=true;}
                        Err(_)=>{tokio::select! { _=retry.tick()=>{}, event=events.next()=>{if handle_offline(event, &mut size)? {break;}} }}
                    }
                }
                event=events.next()=>{if handle_offline(event, &mut size)? {break;}}
            }
            continue;
        };
        enum Next {
            Output(std::result::Result<Option<StreamEvent>, herdr_client_lib::client::Error>),
            Notification(
                std::result::Result<Option<serde_json::Value>, herdr_client_lib::client::Error>,
            ),
            Host(Option<io::Result<Event>>),
            Retry,
        }
        let next = tokio::select! {
            _=&mut interrupt=>break,
            output=async {match &mut current.stream {Some(stream)=>stream.next().await,None=>std::future::pending().await}}=>Next::Output(output),
            notification=current.events.next()=>Next::Notification(notification),
            event=events.next()=>Next::Host(event),
            _=retry.tick(), if current.stream.is_none()=>Next::Retry,
        };
        match next {
            Next::Output(Ok(Some(StreamEvent::Data(bytes)))) => {
                vt.write(&bytes);
                draw(&mut screen, &mut vt)?;
                if current
                    .stream
                    .as_ref()
                    .unwrap()
                    .consumed(bytes.len() as u32)
                    .await
                    .is_err()
                {
                    connection = None;
                }
            }
            Next::Output(Ok(Some(StreamEvent::Closed(_)) | None)) => {
                current.stream = None;
                if !current.record.keep {
                    break;
                }
            }
            Next::Output(Err(_)) | Next::Notification(Err(_) | Ok(None)) => {
                connection = None;
            }
            Next::Notification(Ok(Some(event))) => match current.notification(event).await {
                Ok(changed) => {
                    if changed {
                        let dimensions = current.size();
                        vt.resize(dimensions.0, dimensions.1);
                        dirty = true;
                    }
                }
                Err(_) => connection = None,
            },
            Next::Retry => {
                if current.record.state == "finished" {
                    break;
                }
                if current.attach(size).await.is_ok() {
                    let dimensions = current.size();
                    vt = vt::Vt::new(dimensions.0, dimensions.1);
                    dirty = true;
                }
            }
            Next::Host(event) => {
                let Some(event) = event.transpose()? else {
                    break;
                };
                if is_quit(&event) {
                    break;
                }
                if let Event::Resize(cols, rows) = event {
                    size = (cols.max(1), rows.max(1));
                }
                if let Event::FocusLost = event {
                    focused = false;
                }
                if let Event::FocusGained = event {
                    focused = true;
                }
                let Some(stream) = current.stream.as_ref() else {
                    continue;
                };
                let result = match event {
                    Event::Key(key) => {
                        vt.scroll_to_bottom();
                        dirty = true;
                        let bytes = input::key(&mut vt, key);
                        if bytes.is_empty() {
                            Ok(())
                        } else {
                            stream.try_input(bytes)
                        }
                    }
                    Event::Paste(text) => stream.try_input(vt.paste_bytes(&text)),
                    Event::Resize(_, _) if focused => stream.resize(size.0, size.1).await,
                    Event::FocusGained => {
                        stream.resize(size.0, size.1).await?;
                        stream.focus(true).await
                    }
                    Event::FocusLost => stream.focus(false).await,
                    Event::Mouse(mouse) => {
                        let bytes = input::mouse(&mut vt, mouse);
                        dirty = true;
                        if bytes.is_empty() {
                            Ok(())
                        } else {
                            stream.try_input(bytes)
                        }
                    }
                    _ => Ok(()),
                };
                if result.is_err() {
                    connection = None;
                }
            }
        }
    }
    if let Some(current) = connection
        && let Some(stream) = current.stream
    {
        let _ = stream.detach().await;
    }
    Ok(())
}
fn is_quit(event: &Event) -> bool {
    matches!(event,Event::Key(key) if key.kind!=event::KeyEventKind::Release && key.code==KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL))
}
fn handle_offline(event: Option<io::Result<Event>>, size: &mut (u16, u16)) -> io::Result<bool> {
    let Some(event) = event.transpose()? else {
        return Ok(true);
    };
    if let Event::Resize(cols, rows) = event {
        *size = (cols.max(1), rows.max(1));
    }
    Ok(is_quit(&event))
}

fn draw(screen: &mut Terminal<CrosstermBackend<io::Stdout>>, vt: &mut vt::Vt) -> io::Result<()> {
    let snapshot = vt.frame();
    screen.draw(|frame| {
        let area = frame.area();
        let buffer = frame.buffer_mut();
        for (y, row) in snapshot.rows.iter().take(area.height as usize).enumerate() {
            let mut tail = false;
            for (x, cell) in row.iter().take(area.width as usize).enumerate() {
                if tail {
                    tail = false;
                    continue;
                }
                let mut style = Style::default()
                    .fg(color(cell.fg))
                    .bg(color(cell.bg.unwrap_or(snapshot.bg)));
                for (enabled, modifier) in [
                    (cell.bold, Modifier::BOLD),
                    (cell.faint, Modifier::DIM),
                    (cell.italic, Modifier::ITALIC),
                    (cell.underline, Modifier::UNDERLINED),
                    (cell.strike, Modifier::CROSSED_OUT),
                ] {
                    if enabled {
                        style = style.add_modifier(modifier);
                    }
                }
                buffer[(x as u16, y as u16)]
                    .set_symbol(if cell.text.is_empty() {
                        " "
                    } else {
                        &cell.text
                    })
                    .set_style(style);
                tail = cell.wide;
            }
        }
        if let Some((x, y)) = snapshot
            .cursor
            .filter(|(x, y)| *x < area.width && *y < area.height)
        {
            frame.set_cursor_position((x, y));
        }
    })?;
    Ok(())
}
fn color(rgb: vt::Rgb) -> Color {
    Color::Rgb(rgb.0, rgb.1, rgb.2)
}
