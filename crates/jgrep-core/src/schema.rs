use std::collections::{BTreeMap, BTreeSet};

use jaq_json::Val;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSummary {
    pub path: String,
    pub types: Vec<String>,
    pub count: usize,
}

#[derive(Default)]
struct FieldStats {
    types: BTreeSet<&'static str>,
    count: usize,
}

pub fn infer_schema(values: &[Val], max_documents: usize) -> Vec<FieldSummary> {
    let mut fields = BTreeMap::<String, FieldStats>::new();
    for value in values.iter().take(max_documents) {
        visit_value(value, "", &mut fields);
    }

    fields
        .into_iter()
        .map(|(path, stats)| FieldSummary {
            path,
            types: stats.types.into_iter().map(str::to_owned).collect(),
            count: stats.count,
        })
        .collect()
}

fn visit_value(value: &Val, path: &str, fields: &mut BTreeMap<String, FieldStats>) {
    if !path.is_empty() {
        let stats = fields.entry(path.to_owned()).or_default();
        stats.types.insert(kind(value));
        stats.count += 1;
    }

    match value {
        Val::Obj(object) => {
            for (key, value) in object.iter() {
                let child_path = join_path(path, &key_segment(key));
                visit_value(value, &child_path, fields);
            }
        }
        Val::Arr(values) => {
            let child_path = if path.is_empty() {
                "[]".to_owned()
            } else {
                format!("{path}[]")
            };
            for value in values.iter() {
                visit_value(value, &child_path, fields);
            }
        }
        _ => {}
    }
}

fn join_path(parent: &str, segment: &str) -> String {
    if parent.is_empty() {
        segment.to_owned()
    } else {
        format!("{parent}.{segment}")
    }
}

fn key_segment(key: &Val) -> String {
    match key {
        Val::TStr(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        other => crate::output::format_value(other, false).unwrap_or_else(|_| "<key>".to_owned()),
    }
}

fn kind(value: &Val) -> &'static str {
    match value {
        Val::Null => "null",
        Val::Bool(_) => "boolean",
        Val::Num(_) => "number",
        Val::BStr(_) => "bytes",
        Val::TStr(_) => "string",
        Val::Arr(_) => "array",
        Val::Obj(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_values(input: &[u8]) -> Vec<Val> {
        crate::input::parse_many(crate::input::InputKind::Json, input)
            .unwrap()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn infers_nested_and_array_fields() {
        let values = parse_values(
            br#"{"name":"Alice","tags":["admin"],"profile":{"age":30}}
{"name":"Bob","tags":["user"],"profile":{"age":15}}
"#,
        );

        let fields = infer_schema(&values, 500);

        assert!(fields
            .iter()
            .any(|f| f.path == "name" && f.types == ["string"] && f.count == 2));
        assert!(fields
            .iter()
            .any(|f| f.path == "profile.age" && f.types == ["number"]));
        assert!(fields
            .iter()
            .any(|f| f.path == "tags[]" && f.types == ["string"]));
    }
}
