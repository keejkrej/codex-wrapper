use std::fs;
use std::path::Path;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

const MAX_KEYBINDING_VALUE_LENGTH: usize = 64;
const MAX_KEYBINDING_WHEN_LENGTH: usize = 256;
const MAX_SCRIPT_ID_LENGTH: usize = 24;
const MAX_KEYBINDINGS_COUNT: usize = 256;

const STATIC_COMMANDS: &[&str] = &[
    "terminal.toggle",
    "terminal.split",
    "terminal.new",
    "terminal.close",
    "diff.toggle",
    "chat.new",
    "chat.newLocal",
    "editor.openFavorite",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RawKeybindingRule {
    key: String,
    command: String,
    #[serde(default)]
    when: Option<String>,
}

#[derive(Clone, Debug)]
enum WhenNode {
    Identifier(String),
    Not(Box<WhenNode>),
    And(Box<WhenNode>, Box<WhenNode>),
    Or(Box<WhenNode>, Box<WhenNode>),
}

impl WhenNode {
    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Identifier(name) => json!({ "type": "identifier", "name": name }),
            Self::Not(node) => json!({ "type": "not", "node": node.to_json() }),
            Self::And(left, right) => {
                json!({ "type": "and", "left": left.to_json(), "right": right.to_json() })
            }
            Self::Or(left, right) => {
                json!({ "type": "or", "left": left.to_json(), "right": right.to_json() })
            }
        }
    }
}

pub(crate) fn keybindings_config_path(cwd: &Path) -> String {
    crate::util::normalize_path(&cwd.join(".t3code-keybindings.json"))
}

pub(crate) fn load_resolved_keybindings(
    cwd: &Path,
) -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>)> {
    let path = cwd.join(".t3code-keybindings.json");
    let raw_rules = read_raw_keybinding_rules(&path)?;
    Ok(compile_rules(raw_rules))
}

pub(crate) fn upsert_keybinding(
    cwd: &Path,
    key: String,
    command: String,
    when: Option<String>,
) -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>)> {
    let path = cwd.join(".t3code-keybindings.json");
    let mut raw_rules = read_raw_keybinding_rules(&path).unwrap_or_default();
    raw_rules.retain(|rule| rule.command != command);
    raw_rules.push(RawKeybindingRule { key, command, when });
    if raw_rules.len() > MAX_KEYBINDINGS_COUNT {
        return Err(anyhow!("Too many keybindings"));
    }
    let contents = serde_json::to_string_pretty(&raw_rules)?;
    fs::write(path, contents)?;
    load_resolved_keybindings(cwd)
}

fn read_raw_keybinding_rules(path: &Path) -> Result<Vec<RawKeybindingRule>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(path)?;
    let rules = serde_json::from_str::<Vec<RawKeybindingRule>>(&contents)?;
    Ok(rules)
}

fn compile_rules(
    raw_rules: Vec<RawKeybindingRule>,
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let mut resolved = Vec::new();
    let mut issues = Vec::new();

    for (index, rule) in raw_rules.into_iter().enumerate() {
        match compile_rule(&rule) {
            Ok(value) => resolved.push(value),
            Err(error) => issues.push(json!({
                "kind": "keybindings.invalid-entry",
                "message": error.to_string(),
                "index": index
            })),
        }
    }

    (resolved, issues)
}

fn compile_rule(rule: &RawKeybindingRule) -> Result<serde_json::Value> {
    if rule.key.trim().is_empty() || rule.key.len() > MAX_KEYBINDING_VALUE_LENGTH {
        return Err(anyhow!("Invalid keybinding key"));
    }
    if !is_valid_command(&rule.command) {
        return Err(anyhow!("Invalid keybinding command"));
    }
    if let Some(when) = rule.when.as_ref() {
        if when.trim().is_empty() || when.len() > MAX_KEYBINDING_WHEN_LENGTH {
            return Err(anyhow!("Invalid keybinding when clause"));
        }
    }

    let shortcut = parse_shortcut(&rule.key)?;
    let when_ast = match rule.when.as_deref() {
        Some(value) => Some(parse_when_expression(value)?.to_json()),
        None => None,
    };

    Ok(json!({
        "command": rule.command,
        "shortcut": shortcut,
        "whenAst": when_ast
    }))
}

fn is_valid_command(command: &str) -> bool {
    if STATIC_COMMANDS.contains(&command) {
        return true;
    }
    let Some(script_id) = command
        .strip_prefix("script.")
        .and_then(|value| value.strip_suffix(".run"))
    else {
        return false;
    };
    !script_id.is_empty()
        && script_id.len() <= MAX_SCRIPT_ID_LENGTH
        && script_id
            .chars()
            .all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-')
        && script_id
            .chars()
            .next()
            .map(|char| char.is_ascii_lowercase() || char.is_ascii_digit())
            .unwrap_or(false)
}

fn parse_shortcut(input: &str) -> Result<serde_json::Value> {
    let normalized = input.trim().to_lowercase();
    let raw_tokens = normalized.split('+').collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < raw_tokens.len() {
        let token = raw_tokens[index];
        if token.is_empty() {
            tokens.push("+");
        } else {
            tokens.push(token);
        }
        index += 1;
    }

    let mut meta = false;
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut mod_key = false;
    let mut key = None::<String>;

    for token in tokens {
        match token {
            "mod" => mod_key = true,
            "ctrl" | "control" => ctrl = true,
            "meta" | "cmd" | "command" => meta = true,
            "alt" | "option" => alt = true,
            "shift" => shift = true,
            "esc" => key = Some("escape".to_string()),
            "space" => key = Some(" ".to_string()),
            other => key = Some(other.to_string()),
        }
    }

    let key = key.ok_or_else(|| anyhow!("Missing keybinding key token"))?;
    Ok(json!({
        "key": key,
        "metaKey": meta,
        "ctrlKey": ctrl,
        "shiftKey": shift,
        "altKey": alt,
        "modKey": mod_key
    }))
}

#[derive(Clone, Debug, PartialEq)]
enum WhenToken {
    Identifier(String),
    Not,
    And,
    Or,
    LParen,
    RParen,
}

fn parse_when_expression(input: &str) -> Result<WhenNode> {
    let tokens = tokenize_when(input)?;
    let mut parser = WhenParser { tokens, index: 0 };
    let node = parser.parse_or()?;
    if parser.index != parser.tokens.len() {
        return Err(anyhow!("Unexpected trailing when-clause tokens"));
    }
    Ok(node)
}

fn tokenize_when(input: &str) -> Result<Vec<WhenToken>> {
    let chars = input.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        match chars[index] {
            ' ' | '\t' | '\r' | '\n' => index += 1,
            '!' => {
                tokens.push(WhenToken::Not);
                index += 1;
            }
            '&' if chars.get(index + 1) == Some(&'&') => {
                tokens.push(WhenToken::And);
                index += 2;
            }
            '|' if chars.get(index + 1) == Some(&'|') => {
                tokens.push(WhenToken::Or);
                index += 2;
            }
            '(' => {
                tokens.push(WhenToken::LParen);
                index += 1;
            }
            ')' => {
                tokens.push(WhenToken::RParen);
                index += 1;
            }
            ch if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' => {
                let start = index;
                index += 1;
                while index < chars.len()
                    && (chars[index].is_ascii_alphanumeric()
                        || chars[index] == '_'
                        || chars[index] == '-'
                        || chars[index] == '.')
                {
                    index += 1;
                }
                tokens.push(WhenToken::Identifier(
                    chars[start..index].iter().collect::<String>(),
                ));
            }
            _ => return Err(anyhow!("Invalid when-clause syntax")),
        }
    }
    Ok(tokens)
}

struct WhenParser {
    tokens: Vec<WhenToken>,
    index: usize,
}

impl WhenParser {
    fn parse_or(&mut self) -> Result<WhenNode> {
        let mut node = self.parse_and()?;
        while self.peek() == Some(&WhenToken::Or) {
            self.index += 1;
            let right = self.parse_and()?;
            node = WhenNode::Or(Box::new(node), Box::new(right));
        }
        Ok(node)
    }

    fn parse_and(&mut self) -> Result<WhenNode> {
        let mut node = self.parse_not()?;
        while self.peek() == Some(&WhenToken::And) {
            self.index += 1;
            let right = self.parse_not()?;
            node = WhenNode::And(Box::new(node), Box::new(right));
        }
        Ok(node)
    }

    fn parse_not(&mut self) -> Result<WhenNode> {
        if self.peek() == Some(&WhenToken::Not) {
            self.index += 1;
            return Ok(WhenNode::Not(Box::new(self.parse_not()?)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<WhenNode> {
        match self.next() {
            Some(WhenToken::Identifier(name)) => Ok(WhenNode::Identifier(name.clone())),
            Some(WhenToken::LParen) => {
                let node = self.parse_or()?;
                match self.next() {
                    Some(WhenToken::RParen) => Ok(node),
                    _ => Err(anyhow!("Missing closing parenthesis in when clause")),
                }
            }
            _ => Err(anyhow!("Invalid when clause")),
        }
    }

    fn peek(&self) -> Option<&WhenToken> {
        self.tokens.get(self.index)
    }

    fn next(&mut self) -> Option<&WhenToken> {
        let token = self.tokens.get(self.index);
        if token.is_some() {
            self.index += 1;
        }
        token
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mod_shortcut() {
        let shortcut = parse_shortcut("mod+shift+k").unwrap();
        assert_eq!(shortcut["modKey"], json!(true));
        assert_eq!(shortcut["shiftKey"], json!(true));
        assert_eq!(shortcut["key"], json!("k"));
    }

    #[test]
    fn parses_when_expression() {
        let ast = parse_when_expression("terminalOpen && !terminalFocus").unwrap();
        assert_eq!(ast.to_json()["type"], json!("and"));
    }
}
