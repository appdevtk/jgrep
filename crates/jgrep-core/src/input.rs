use std::path::{Path, PathBuf};

use jaq_json::Val;
#[cfg(feature = "yaml")]
use serde::Deserialize;

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
        Some("json") => Some(InputKind::Json),
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
        let first = bytes
            .iter()
            .copied()
            .find(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'));

        match first {
            Some(b'{' | b'[' | b'"' | b't' | b'f' | b'n' | b'-' | b'0'..=b'9') => InputKind::Json,
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
