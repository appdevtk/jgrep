use std::path::{Path, PathBuf};

use jaq_json::Val;
#[cfg(feature = "yaml")]
use serde::Deserialize;

pub const MAX_JSON_DEPTH: usize = 128;

/// Bound recursion before entering jaq's parser, including its relaxed JSON
/// syntax (line comments, byte strings and recursively parsed object keys).
pub fn check_json_depth(bytes: &[u8]) -> Result<(), String> {
    let (mut depth, mut quoted, mut escaped, mut comment) = (0usize, false, false, false);
    for &byte in bytes {
        if comment {
            comment = byte != b'\n';
        } else if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'#' => comment = true,
                b'"' => quoted = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > MAX_JSON_DEPTH {
                        return Err(format!("JSON nesting exceeds {MAX_JSON_DEPTH} levels"));
                    }
                }
                b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Json,
    #[cfg(feature = "yaml")]
    Yaml,
}

#[derive(Debug, Clone)]
pub enum Source {
    File(PathBuf),
    Stdin,
}

impl Source {
    pub fn path(&self) -> &Path {
        match self {
            Source::File(path) => path,
            Source::Stdin => Path::new("stdin"),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Source::File(path) => path.display().to_string(),
            Source::Stdin => "stdin".to_owned(),
        }
    }
}

pub fn kind_for_path(path: &Path) -> Option<InputKind> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json" | "ndjson" | "jsonl") => Some(InputKind::Json),
        #[cfg(feature = "yaml")]
        Some("yaml" | "yml") => Some(InputKind::Yaml),
        _ => None,
    }
}

pub fn detect_stdin_kind(bytes: &[u8]) -> InputKind {
    #[cfg(not(feature = "yaml"))]
    {
        let _ = bytes;
        InputKind::Json
    }

    #[cfg(feature = "yaml")]
    {
        let trimmed = trim_ascii_whitespace(bytes);
        match trimmed.first() {
            Some(b'{' | b'[' | b'"') => InputKind::Json,
            Some(b't' | b'f' | b'n' | b'-' | b'0'..=b'9') => {
                // A YAML key such as `true_value:` can start with a valid JSON
                // scalar prefix. Require the entire first token to be JSON.
                let token = trimmed
                    .split(u8::is_ascii_whitespace)
                    .next()
                    .unwrap_or_default();
                if check_json_depth(token).is_err()
                    || jaq_json::read::parse_many(token).all(|value| value.is_ok())
                {
                    InputKind::Json
                } else {
                    InputKind::Yaml
                }
            }
            _ => InputKind::Yaml,
        }
    }
}

pub fn is_streamable_json_line(line: &[u8]) -> bool {
    let trimmed = trim_ascii_whitespace(line);
    matches!(trimmed.first(), Some(b'{'))
        && parse_many(InputKind::Json, trimmed)
            .is_ok_and(|values| !values.is_empty() && values.into_iter().all(|value| value.is_ok()))
}

pub fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map(|idx| idx + 1)
        .unwrap_or(start);
    &bytes[start..end]
}

pub fn parse_many(kind: InputKind, bytes: &[u8]) -> Result<Vec<Result<Val, String>>, String> {
    match kind {
        InputKind::Json => parse_json_many(bytes),
        #[cfg(feature = "yaml")]
        InputKind::Yaml => parse_yaml_many(bytes),
    }
}

fn parse_json_many(bytes: &[u8]) -> Result<Vec<Result<Val, String>>, String> {
    check_json_depth(bytes)?;
    let mut values = Vec::new();
    for value in jaq_json::read::parse_many(bytes) {
        match value {
            Ok(value) => values.push(Ok(value)),
            Err(e) => {
                values.push(Err(e.to_string()));
                break;
            }
        }
    }
    Ok(values)
}

#[cfg(feature = "yaml")]
fn parse_yaml_many(bytes: &[u8]) -> Result<Vec<Result<Val, String>>, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    Ok(yaml_serde::Deserializer::from_str(text)
        .map(|doc| Val::deserialize(doc).map_err(|e| e.to_string()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_depth_before_parse_and_detection() {
        let deep = format!("{}0{}", "[".repeat(20000), "]".repeat(20000));
        assert!(parse_many(InputKind::Json, deep.as_bytes()).is_err());
        assert!(!is_streamable_json_line(
            format!("{{\"a\":{deep}}}").as_bytes()
        ));
        assert!(matches!(
            detect_stdin_kind(format!("0{deep}").as_bytes()),
            InputKind::Json
        ));
    }

    #[test]
    fn depth_guard_understands_comments_strings_and_object_keys() {
        for text in [
            format!("# {}\n{{\"a\":1}}", "[\"".repeat(200)),
            format!("\"{}\"", "[".repeat(200)),
            format!("b\"{}\"", "[".repeat(200)),
            r#"["escaped \" [[[",1]"#.to_owned(),
            format!(
                "{}0{}",
                "[".repeat(MAX_JSON_DEPTH),
                "]".repeat(MAX_JSON_DEPTH)
            ),
        ] {
            assert!(
                parse_many(InputKind::Json, text.as_bytes())
                    .unwrap()
                    .iter()
                    .all(Result::is_ok),
                "{text}"
            );
        }
        // Closers in a comment must not hide recursion below it.
        let hidden = format!(
            "[ # {}\n{}0{}]",
            "]".repeat(200),
            "[".repeat(128),
            "]".repeat(128)
        );
        assert!(check_json_depth(hidden.as_bytes()).is_err());
        let key = format!("{{{}0{}:1}}", "[".repeat(128), "]".repeat(128));
        assert!(parse_many(InputKind::Json, key.as_bytes()).is_err());
    }
}
