//! Herdr's `ui.window_title`, adapted from Herdr `src/config/window_title.rs` at
//! ee5bfb5041bb3e4a49d591c72308aae3c81ecdde.

/// Maximum characters in a rendered outer terminal window title.
const MAX_WINDOW_TITLE_CHARS: usize = 200;

pub fn sanitize(value: &str) -> Option<String> {
    let sanitized = value
        .chars()
        .filter(|ch| !matches!(*ch, '\u{1b}' | '\u{7}' | '\u{9c}') && !ch.is_control())
        .take(MAX_WINDOW_TITLE_CHARS)
        .collect::<String>()
        .trim()
        .to_string();
    (!sanitized.is_empty()).then_some(sanitized)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Hostname,
    Workspace,
    Tab,
    Pane,
    TerminalTitle,
}

impl Token {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "hostname" => Some(Self::Hostname),
            "workspace" => Some(Self::Workspace),
            "tab" => Some(Self::Tab),
            "pane" => Some(Self::Pane),
            "terminal_title" => Some(Self::TerminalTitle),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Literal(String),
    Token(Token),
}

/// Parses a window title format. An empty format disables outer window titles.
pub fn parse(template: &str) -> Result<Option<Vec<Part>>, String> {
    if template.is_empty() {
        return Ok(None);
    }
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for ch in chars.by_ref() {
                    if ch == '}' {
                        closed = true;
                        break;
                    }
                    name.push(ch);
                }
                if !closed {
                    return Err("has an unclosed '{'".to_string());
                }
                let Some(token) = Token::parse(name.trim()) else {
                    return Err(format!("has unknown token '{{{name}}}'"));
                };
                if !literal.is_empty() {
                    parts.push(Part::Literal(std::mem::take(&mut literal)));
                }
                parts.push(Part::Token(token));
            }
            '}' => return Err("has an unmatched '}'".to_string()),
            _ => literal.push(ch),
        }
    }
    if !literal.is_empty() {
        parts.push(Part::Literal(literal));
    }
    Ok((!parts.is_empty()).then_some(parts))
}

/// Fills a parsed format; a missing value leaves its token empty.
pub fn render(parts: &[Part], value: impl Fn(Token) -> Option<String>) -> Option<String> {
    let text: String = parts
        .iter()
        .map(|part| match part {
            Part::Literal(text) => text.clone(),
            Part::Token(token) => value(*token).unwrap_or_default(),
        })
        .collect();
    sanitize(&text)
}

/// This machine's name.
pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .map(|name| name.trim().to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_tokens_and_escapes() {
        let parts = parse("{hostname}: {workspace} {{a}}").unwrap().unwrap();
        let title = render(&parts, |token| match token {
            Token::Hostname => Some("box".into()),
            Token::Workspace => Some("api\u{7}".into()),
            _ => None,
        });
        assert_eq!(title.as_deref(), Some("box: api {a}"));
        assert!(parse("").unwrap().is_none());
        assert!(parse("{nope}").is_err());
        assert!(parse("{workspace").is_err());
    }
}
