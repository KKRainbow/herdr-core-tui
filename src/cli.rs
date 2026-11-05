use crate::{Result, actions, control};
use herdr_client_lib::{sync::Session, transport::HcConfig, view::View};
use serde_json::{Value, json};
use std::time::Duration;
pub struct Options {
    pub config: HcConfig,
    pub machine: String,
    pub instance: Option<String>,
    pub positional: Vec<String>,
}
pub fn parse(args: Vec<String>) -> Result<Options> {
    let mut session = std::env::var("HC_SESSION").ok();
    let mut binary = std::env::var_os("HC_BINARY").unwrap_or_else(|| "hc".into());
    let mut machine = String::new();
    let mut instance = None;
    let mut positional = vec![];
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--session" => session = Some(args.next().ok_or("--session requires a value")?),
            "--hc" => binary = args.next().ok_or("--hc requires a path")?.into(),
            "--machine" => machine = args.next().ok_or("--machine requires a name")?,
            "--instance" => instance = Some(args.next().ok_or("--instance requires a name")?),
            _ => {
                positional.push(arg);
                positional.extend(args);
                break;
            }
        }
    }
    let session = session
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default")
        .to_owned();
    Ok(Options {
        config: HcConfig {
            binary: binary.into(),
            session,
            environment: Default::default(),
        },
        machine,
        instance,
        positional,
    })
}
pub const HELP: &str = "herdr-tui [--session SESSION] [--hc PATH] [--machine MACHINE] [--instance NAME] [TERMINAL]\n\nSession defaults to HC_SESSION or default, like hc.\nWithout TERMINAL, open the Herdr workspace interface. Ctrl+B is the prefix: prefix+? lists keybinds and prefix+q detaches.\nWith TERMINAL, view one terminal; Ctrl+Q quits that viewer.\n\nherdr-tui [--session SESSION] [--machine MACHINE] do COMMAND [JSON]\n  Run a shared structural command without opening a TUI. Targets must be explicit.\nherdr-tui [--session SESSION] --instance NAME view COMMAND [JSON]\n  Control only the named live TUI. Commands: state, pane.focus, tab.focus, space.focus,\n  agent.focus, terminal.focus, zoom, machine, quit. Focus takes {\"id\":\"...\"}.\nherdr-tui [--session SESSION] instances\n  List live TUI instances. No automatic choice when several exist.\n\nExamples:\n  herdr-tui --instance desk\n  herdr-tui do space.new '{\"name\":\"API\",\"folder\":\"/path/to/repo\"}'\n  herdr-tui --instance desk view zoom\n  herdr-tui --session work do help\n\nUse hc for runtime automation: run, send, keys, read, wait, and machine management.\nThe TUI never replaces or upgrades installed app/core binaries.";
pub async fn headless(options: &Options) -> Result<bool> {
    let Some(kind) = options.positional.first().map(String::as_str) else {
        return Ok(false);
    };
    if kind == "instances" {
        if options.positional.len() != 1 {
            return Err("instances takes no arguments".into());
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&control::instances(&options.config.session).await?)?
        );
        return Ok(true);
    }
    if !matches!(kind, "do" | "view") {
        return Ok(false);
    }
    if !(2..=3).contains(&options.positional.len()) {
        return Err("Expected COMMAND and optional JSON object".into());
    }
    let command = &options.positional[1];
    let args: Value = options
        .positional
        .get(2)
        .map(|s| serde_json::from_str(s))
        .transpose()?
        .unwrap_or(json!({}));
    if !args.is_object() {
        return Err("Command arguments must be a JSON object".into());
    }
    let value = if kind == "view" {
        control::call(
            &options.config.session,
            options
                .instance
                .as_deref()
                .ok_or("view commands require --instance NAME")?,
            command,
            args,
        )
        .await?
    } else if command == "help" {
        json!({"help":actions::HELP})
    } else {
        let deadline = Duration::from_secs(10);
        let client = options.config.connect(deadline).await?;
        let mut session = Session::connect(client, [options.machine.clone()], deadline).await?;
        actions::execute(
            &mut session,
            &options.machine,
            &mut View::default(),
            command,
            args,
        )
        .await?
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    if value.get("error").is_some() || value["ok"] == false {
        return Err("Command failed; see response".into());
    }
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_explicit_session_and_target_instance() {
        let o = parse(
            ["--session", "test", "--instance", "a", "view", "zoom"]
                .map(str::to_owned)
                .to_vec(),
        )
        .unwrap();
        assert_eq!(o.config.session, "test");
        assert_eq!(o.instance.as_deref(), Some("a"));
        assert_eq!(o.positional, ["view", "zoom"]);
    }
}
