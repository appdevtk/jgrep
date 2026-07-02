use jaq_json::Val;

use crate::cli::Cli;

const CYAN: &str = "\u{1b}[36m";
const GREEN: &str = "\u{1b}[32m";
const BLUE: &str = "\u{1b}[34m";
const YELLOW: &str = "\u{1b}[33m";
const MAGENTA: &str = "\u{1b}[35m";
const RED: &str = "\u{1b}[31m";
const BOLD_RED: &str = "\u{1b}[1;31m";
const RESET: &str = "\u{1b}[0m";

const DEFAULT_LEVEL_FIELDS: &[&str] = &[
    "log.level",
    "level",
    "severity",
    "severity_text",
    "severityText",
];

pub fn color_for_level(level: &str) -> Option<&'static str> {
    match level.trim().to_ascii_uppercase().as_str() {
        "TRACE" => Some(MAGENTA),
        "DEBUG" => Some(BLUE),
        "INFO" | "INFORMATION" | "NOTICE" => Some(CYAN),
        "WARN" | "WARNING" => Some(YELLOW),
        "ERROR" | "ERR" => Some(RED),
        "FATAL" | "CRITICAL" | "CRIT" | "ALERT" | "EMERGENCY" => Some(BOLD_RED),
        _ => None,
    }
}

pub fn colorize_by_level(text: &str, source: &Val, cli: &Cli) -> Option<String> {
    if !cli.use_level_color() {
        return None;
    }
    let level = level_value(source, cli)?;
    let color = color_for_level(&level)?;
    Some(format!("{color}{text}{RESET}"))
}

fn level_value(source: &Val, cli: &Cli) -> Option<String> {
    if let Some(field) = cli.color_level_field.as_deref().filter(|s| !s.is_empty()) {
        return field_value(source, field).and_then(scalar_text);
    }
    DEFAULT_LEVEL_FIELDS
        .iter()
        .find_map(|field| field_value(source, field).and_then(scalar_text))
}

fn field_value<'a>(value: &'a Val, field: &str) -> Option<&'a Val> {
    if let Val::Obj(map) = value {
        let direct_key = Val::utf8_str(field.as_bytes().to_vec());
        if let Some(value) = map.get(&direct_key) {
            return Some(value);
        }

        let mut current = value;
        for part in field.split('.') {
            let Val::Obj(map) = current else {
                return None;
            };
            let key = Val::utf8_str(part.as_bytes().to_vec());
            current = map.get(&key)?;
        }
        Some(current)
    } else {
        None
    }
}

fn scalar_text(value: &Val) -> Option<String> {
    match value {
        Val::TStr(bytes) | Val::BStr(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Val::Bool(b) => Some(b.to_string()),
        Val::Num(n) => Some(n.to_string()),
        _ => None,
    }
}

pub fn colorize_json(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    let mut chars = json.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        match ch {
            '"' => {
                let start = idx;
                let mut end = idx + 1;
                let mut escaped = false;
                for (i, c) in chars.by_ref() {
                    end = i + c.len_utf8();
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        break;
                    }
                }
                let token = &json[start..end];
                let is_key = json[end..].trim_start().starts_with(':');
                if is_key {
                    out.push_str(CYAN);
                } else {
                    out.push_str(GREEN);
                }
                out.push_str(token);
                out.push_str(RESET);
            }
            '-' | '0'..='9' => {
                let start = idx;
                let mut end = idx + ch.len_utf8();
                while let Some((i, c)) = chars.peek().copied() {
                    if c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-') {
                        chars.next();
                        end = i + c.len_utf8();
                    } else {
                        break;
                    }
                }
                out.push_str(YELLOW);
                out.push_str(&json[start..end]);
                out.push_str(RESET);
            }
            't' if json[idx..].starts_with("true") => {
                out.push_str(MAGENTA);
                out.push_str("true");
                out.push_str(RESET);
                for _ in 0..3 {
                    chars.next();
                }
            }
            'f' if json[idx..].starts_with("false") => {
                out.push_str(MAGENTA);
                out.push_str("false");
                out.push_str(RESET);
                for _ in 0..4 {
                    chars.next();
                }
            }
            'n' if json[idx..].starts_with("null") => {
                out.push_str(MAGENTA);
                out.push_str("null");
                out.push_str(RESET);
                for _ in 0..3 {
                    chars.next();
                }
            }
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_levels() {
        assert_eq!(color_for_level("TRACE"), Some(MAGENTA));
        assert_eq!(color_for_level("DEBUG"), Some(BLUE));
        assert_eq!(color_for_level("INFO"), Some(CYAN));
        assert_eq!(color_for_level("WARN"), Some(YELLOW));
        assert_eq!(color_for_level("ERROR"), Some(RED));
        assert_eq!(color_for_level("FATAL"), Some(BOLD_RED));
        assert_eq!(color_for_level("UNKNOWN"), None);
    }
}
