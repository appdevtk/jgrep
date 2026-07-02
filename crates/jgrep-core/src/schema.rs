use std::collections::{BTreeMap, BTreeSet};

use jaq_json::Val;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSummary {
    pub path: String,
    pub types: Vec<String>,
    pub type_counts: Vec<(String, usize)>,
    pub count: usize,
    pub documents: usize,
    pub optional: bool,
    pub examples: Vec<String>,
}

#[derive(Default)]
struct FieldStats {
    type_counts: BTreeMap<&'static str, usize>,
    documents: BTreeSet<usize>,
    examples: BTreeSet<String>,
    count: usize,
}

pub fn infer_schema(values: &[Val], max_documents: usize, max_depth: usize) -> Vec<FieldSummary> {
    let mut fields = BTreeMap::<String, FieldStats>::new();
    let sampled = values.len().min(max_documents);
    for (document, value) in values.iter().take(max_documents).enumerate() {
        visit_value(value, "", document, 0, max_depth, &mut fields);
    }

    fields
        .into_iter()
        .map(|(path, stats)| FieldSummary {
            path,
            types: stats
                .type_counts
                .keys()
                .copied()
                .map(str::to_owned)
                .collect(),
            type_counts: stats
                .type_counts
                .into_iter()
                .map(|(kind, count)| (kind.to_owned(), count))
                .collect(),
            count: stats.count,
            documents: stats.documents.len(),
            optional: stats.documents.len() < sampled,
            examples: stats.examples.into_iter().take(3).collect(),
        })
        .collect()
}

fn visit_value(
    value: &Val,
    path: &str,
    document: usize,
    depth: usize,
    max_depth: usize,
    fields: &mut BTreeMap<String, FieldStats>,
) {
    if !path.is_empty() {
        let stats = fields.entry(path.to_owned()).or_default();
        *stats.type_counts.entry(kind(value)).or_default() += 1;
        stats.documents.insert(document);
        stats.count += 1;
        if stats.examples.len() < 3 && !matches!(value, Val::Arr(_) | Val::Obj(_)) {
            if let Ok(example) = crate::output::format_value(value, false) {
                stats.examples.insert(example);
            }
        }
    }

    if depth >= max_depth {
        return;
    }

    match value {
        Val::Obj(object) => {
            for (key, value) in object.iter() {
                let child_path = join_path(path, &key_segment(key));
                visit_value(value, &child_path, document, depth + 1, max_depth, fields);
            }
        }
        Val::Arr(values) => {
            let child_path = if path.is_empty() {
                "[]".to_owned()
            } else {
                format!("{path}[]")
            };
            for value in values.iter() {
                visit_value(value, &child_path, document, depth + 1, max_depth, fields);
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

        let fields = infer_schema(&values, 500, 8);

        assert!(fields.iter().any(|f| f.path == "name"
            && f.types == ["string"]
            && f.count == 2
            && f.documents == 2
            && !f.optional
            && f.examples == ["Alice", "Bob"]));
        assert!(fields
            .iter()
            .any(|f| f.path == "profile.age" && f.types == ["number"]));
        assert!(fields
            .iter()
            .any(|f| f.path == "tags[]" && f.types == ["string"]));
    }

    #[test]
    fn marks_optional_fields_and_honors_max_depth() {
        let values = parse_values(
            br#"{"profile":{"age":30},"enabled":true}
{"enabled":false}
"#,
        );

        let shallow = infer_schema(&values, 500, 1);
        assert!(shallow.iter().any(|f| f.path == "profile"));
        assert!(!shallow.iter().any(|f| f.path == "profile.age"));

        let fields = infer_schema(&values, 500, 8);
        let age = fields.iter().find(|f| f.path == "profile.age").unwrap();
        assert!(age.optional);
        assert_eq!(age.documents, 1);
        assert_eq!(age.type_counts, [("number".to_owned(), 1)]);
    }
}
