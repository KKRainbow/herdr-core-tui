mod input;
mod vt;
use crossterm::{
    cursor,
    event::{self, Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use herdr_client_lib::{client::StreamEvent, transport::HcConfig};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    style::{Color, Modifier, Style},
};
use serde_json::json;
use std::{
    io::{self, stdout},
    time::Duration,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
struct Host;
impl Host {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let guard = Self;
        execute!(
            stdout(),
            EnterAlternateScreen,
            event::EnableBracketedPaste,
            event::EnableFocusChange,
            event::EnableMouseCapture
        )?;
        Ok(guard)
    }
}
fn restore() {
    let _ = execute!(
        stdout(),
        event::DisableMouseCapture,
        event::DisableFocusChange,
        event::DisableBracketedPaste,
        cursor::Show,
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
        println!(
            "herder-tui --session SESSION TERMINAL\nAttach to an existing core terminal. Ctrl+Q detaches.\nSet HC_BINARY to select hc."
        );
        return Ok(());
    }
    if args == ["--version"] {
        println!("herder-tui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.len() != 3 || args[0] != "--session" || args[1].is_empty() {
        return Err("usage: herder-tui --session SESSION TERMINAL".into());
    }
    let config = HcConfig {
        binary: std::env::var_os("HC_BINARY")
            .unwrap_or_else(|| "hc".into())
            .into(),
        session: args[1].clone(),
        environment: Default::default(),
    };
    let client = config.connect(Duration::from_secs(10)).await?;
    let (cols, rows) = terminal::size()?;
    let (_, mut stream) = client
        .attach(
            json!({"terminal":args[2],"cols":cols,"rows":rows}),
            Duration::from_secs(10),
        )
        .await?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
    let _host = Host::enter()?;
    let mut screen = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut vt = vt::Vt::new(cols, rows);
    let mut events = EventStream::new();
    stream.focus(true).await?;
    loop {
        draw(&mut screen, &mut vt)?;
        tokio::select! {
            output = stream.next() => match output? {
                Some(StreamEvent::Data(bytes)) => {vt.write(&bytes); draw(&mut screen,&mut vt)?; stream.consumed(u32::try_from(bytes.len())?).await?;}
                Some(StreamEvent::Closed(_)) | None => break,
            },
            event = events.next() => match event.transpose()? {
                None => break,
                Some(Event::Key(key)) if key.code==KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) => break,
                Some(Event::Key(key)) => {vt.scroll_to_bottom(); let bytes=input::key(&mut vt,key); if !bytes.is_empty(){stream.input(bytes).await?;}}
                Some(Event::Paste(text)) => stream.input(vt.paste_bytes(&text)).await?,
                Some(Event::Resize(cols,rows)) => {vt.resize(cols,rows); stream.resize(cols,rows).await?;}
                Some(Event::FocusGained) => stream.focus(true).await?,
                Some(Event::FocusLost) => stream.focus(false).await?,
                Some(Event::Mouse(mouse)) => {let bytes=input::mouse(&mut vt,mouse); if !bytes.is_empty(){stream.input(bytes).await?;}}
            }
        }
    }
    stream.detach().await?;
    Ok(())
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
