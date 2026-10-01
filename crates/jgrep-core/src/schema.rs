use std::collections::{BTreeMap, BTreeSet};

use jaq_json::Val;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSummary {
    pub path: String,
    pub jq_path: String,
    pub types: Vec<String>,
    pub type_counts: Vec<(String, usize)>,
    pub count: usize,
    pub documents: usize,
    pub optional: bool,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum PathSegment {
    Key { label: String, lookup: String },
    ArrayElement,
}

fn display_path(path: &[PathSegment]) -> String {
    let mut display = String::new();
    for segment in path {
        match segment {
            PathSegment::Key { label, .. } => {
                if !display.is_empty() {
                    display.push('.');
                }
                display.push_str(label);
            }
            PathSegment::ArrayElement => display.push_str("[]"),
        }
    }
    display
}

#[derive(Default)]
struct FieldStats {
    type_counts: BTreeMap<&'static str, usize>,
    documents: BTreeSet<usize>,
    examples: BTreeSet<String>,
    count: usize,
}

pub fn infer_schema(
    values: &[Val],
    max_documents: usize,
    max_depth: usize,
) -> Result<Vec<FieldSummary>, String> {
    let mut fields = BTreeMap::<Vec<PathSegment>, FieldStats>::new();
    let sampled = values.len().min(max_documents);
    for (document, value) in values.iter().take(max_documents).enumerate() {
        visit_value(value, &[], document, 0, max_depth, &mut fields)?;
    }

    Ok(fields
        .into_iter()
        .map(|(path, stats)| FieldSummary {
            path: display_path(&path),
            jq_path: path
                .iter()
                .enumerate()
                .map(|(index, segment)| match segment {
                    PathSegment::Key { lookup, .. } => lookup.as_str(),
                    PathSegment::ArrayElement if index == 0 => ".[]",
                    PathSegment::ArrayElement => "[]",
                })
                .collect(),
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
        .collect())
}

fn visit_value(
    value: &Val,
    path: &[PathSegment],
    document: usize,
    depth: usize,
    max_depth: usize,
    fields: &mut BTreeMap<Vec<PathSegment>, FieldStats>,
) -> Result<(), String> {
    if !path.is_empty() {
        let stats = fields.entry(path.to_vec()).or_default();
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
        return Ok(());
    }

    match value {
        Val::Obj(object) => {
            for (key, value) in object.iter() {
                let mut child_path = path.to_vec();
                child_path.push(key_segment(key)?);
                visit_value(value, &child_path, document, depth + 1, max_depth, fields)?;
            }
        }
        Val::Arr(values) => {
            let mut child_path = path.to_vec();
            child_path.push(PathSegment::ArrayElement);
            for value in values.iter() {
                visit_value(value, &child_path, document, depth + 1, max_depth, fields)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn key_segment(key: &Val) -> Result<PathSegment, String> {
    let label = match key {
        Val::TStr(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        other => crate::output::format_value(other, false).unwrap_or_else(|_| "<key>".to_owned()),
    };
    let mut chars = label.chars();
    let identifier = matches!(key, Val::TStr(_))
        && matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric());
    let lookup = if identifier {
        format!(".{label}")
    } else {
        let literal = crate::output::format_json_value(key, false).map_err(|error| {
            format!("schema completion unavailable: invalid key encoding ({error})")
        })?;
        format!(".[{literal}]")
    };
    Ok(PathSegment::Key { label, lookup })
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

        let fields = infer_schema(&values, 500, 8).unwrap();

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

        let shallow = infer_schema(&values, 500, 1).unwrap();
        assert!(shallow.iter().any(|f| f.path == "profile"));
        assert!(!shallow.iter().any(|f| f.path == "profile.age"));

        let fields = infer_schema(&values, 500, 8).unwrap();
        let age = fields.iter().find(|f| f.path == "profile.age").unwrap();
        assert!(age.optional);
        assert_eq!(age.documents, 1);
        assert_eq!(age.type_counts, [("number".to_owned(), 1)]);
    }

    #[test]
    fn literal_and_nested_paths_remain_distinct_and_executable() {
        let values = parse_values(br#"{"a.b":1,"a":{"b":2},"tags":[{"name":"ok"}],"":3}"#);
        let fields = infer_schema(&values, 10, 8).unwrap();
        let dotted = fields
            .iter()
            .filter(|field| field.path == "a.b")
            .collect::<Vec<_>>();
        assert_eq!(dotted.len(), 2);
        let results = dotted
            .iter()
            .map(|field| {
                let matcher = crate::matcher::Matcher::compile(&field.jq_path).unwrap();
                crate::output::format_value(&matcher.apply(values[0].clone()).unwrap()[0], false)
                    .unwrap()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(results, BTreeSet::from(["1".to_owned(), "2".to_owned()]));
        let array = fields
            .iter()
            .find(|field| field.path == "tags[].name")
            .unwrap();
        assert_eq!(array.jq_path, ".tags[].name");
        let result = crate::matcher::Matcher::compile(&array.jq_path)
            .unwrap()
            .apply(values[0].clone())
            .unwrap();
        assert_eq!(
            crate::output::format_value(&result[0], false).unwrap(),
            "ok"
        );
        assert!(fields
            .iter()
            .any(|field| field.path.is_empty() && field.jq_path == ".[\"\"]"));
    }

    #[test]
    fn invalid_utf8_keys_report_a_controlled_schema_error() {
        for input in [b"{\"\xff\":1}".as_slice(), b"{[\"\xff\"]:1}".as_slice()] {
            let values = parse_values(input);
            assert!(infer_schema(&values, 10, 8)
                .unwrap_err()
                .contains("invalid key encoding"));
        }
    }
}
