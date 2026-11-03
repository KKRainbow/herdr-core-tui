use herdr_client_lib::protocol::Terminal;

const AGENT_NAMES: &[&str] = &[
    "agy",
    "aider",
    "amp",
    "amp-local",
    "antigravity",
    "antigravity-cli",
    "claude",
    "claude-code",
    "cline",
    ".cline",
    "codex",
    "command-code",
    "copilot",
    "crush",
    "cursor",
    "cursor-agent",
    "devin",
    "devin-cli",
    "droid",
    "dsh-tui",
    "gemini",
    "ghcs",
    "github-copilot",
    "grok",
    "grok-build",
    "hermes",
    "hermes-agent",
    "kilo",
    "kilo-code",
    "kimi",
    "kimi-code",
    "kiro",
    "kiro-cli",
    "letta",
    "letta-code",
    "maki",
    "mastra-code",
    "mastracode",
    "muse",
    "muse-cli",
    "muse-code",
    "omp",
    "open-code",
    "opencode",
    "opencode2",
    "pi",
    "pig",
    "prime-agent",
    "qoder",
    "qodercn",
    "qodercli",
    "qoderclicn",
    "qwen",
    "qwen-code",
];

fn basename(name: &str) -> &str {
    let name = name.trim().rsplit(['/', '\\']).next().unwrap_or(name);
    name.rsplit_once('.')
        .filter(|(_, suffix)| {
            ["exe", "cmd", "ps1"]
                .iter()
                .any(|s| s.eq_ignore_ascii_case(suffix))
        })
        .map_or(name, |(stem, _)| stem)
}

fn known(name: &str) -> Option<&str> {
    let name = basename(name);
    AGENT_NAMES
        .iter()
        .any(|agent| agent.eq_ignore_ascii_case(name))
        .then_some(name)
}

fn program(argv: &[String]) -> Option<&str> {
    let executable = basename(argv.first()?);
    if let Some(agent) = known(executable) {
        return Some(agent);
    }
    if !["node", "bun", "deno", "python", "python3"]
        .iter()
        .any(|runtime| runtime.eq_ignore_ascii_case(executable))
    {
        return None;
    }
    let script = argv.get(1)?.as_str();
    if script.starts_with('-') {
        return None;
    }
    let script = basename(script);
    let stem = script
        .rsplit_once('.')
        .filter(|(_, suffix)| matches!(*suffix, "js" | "mjs" | "cjs" | "ts" | "py"))
        .map_or(script, |(stem, _)| stem);
    known(stem)
}

/// A recognized agent report or current program; terminal names and status alone are not proof.
pub fn label(record: &Terminal) -> Option<&str> {
    if record.state == "finished" {
        return None;
    }
    if let Some(app) = record
        .status
        .as_ref()
        .map(|status| status.app.trim())
        .filter(|app| !app.is_empty())
    {
        return known(app);
    }
    if let Some(foreground) = &record.foreground {
        return program(&foreground.argv).or_else(|| known(&foreground.name));
    }
    program(&record.argv)
}
