pub fn path_filter(path: &str) -> Result<String, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("path must not be empty".to_owned());
    }
    Ok(path_to_jq(path))
}

#[cfg(feature = "explore")]
pub fn expression_to_jq(expression: &str) -> Result<String, String> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Ok(".".to_owned());
    }

    if expression.starts_with("select(") {
        return Ok(expression.to_owned());
    }

    if expression.starts_with('.') {
        if let Some((field, op, value)) = split_condition(expression) {
            // Only normalize a field comparison, not arbitrary jq pipelines or
            // assignments. Quoted completion keys remain literal lookups.
            if is_field_lookup(field.trim()) && is_comparison_value(value.trim()) {
                let op = if op == "=" { "==" } else { op };
                return Ok(format!(
                    "select({} {op} {}) | .",
                    field.trim(),
                    literal(value.trim())
                ));
            }
        }
        return Ok(expression.to_owned());
    }

    if contains_condition_operator(expression) {
        apply_where_filters(".".to_owned(), &[expression.to_owned()])
    } else {
        path_filter(expression)
    }
}

#[cfg(feature = "explore")]
pub fn output_expression_to_jq(expression: &str) -> Result<String, String> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Ok(".".to_owned());
    }

    if expression.starts_with('.')
        || expression.starts_with("select(")
        || expression.starts_with('{')
        || expression.starts_with('[')
    {
        return Ok(expression.to_owned());
    }

    path_filter(expression)
}

pub fn apply_where_filters(base_filter: String, filters: &[String]) -> Result<String, String> {
    if filters.is_empty() {
        return Ok(base_filter);
    }

    Ok(format!(
        "{} | {base_filter}",
        where_filters_to_select(filters)?
    ))
}

pub fn where_filters_to_select(filters: &[String]) -> Result<String, String> {
    let mut conditions = Vec::with_capacity(filters.len());
    for filter in filters {
        conditions.push(where_condition(filter)?);
    }

    Ok(format!("select({})", conditions.join(" and ")))
}

#[cfg(feature = "explore")]
fn contains_condition_operator(input: &str) -> bool {
    split_condition(input).is_some()
}

// Ignore operators inside quoted field names and values. Prefer the first
// operator, so a value containing another operator is not mistaken for a field.
fn split_condition(input: &str) -> Option<(&str, &str, &str)> {
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in input.char_indices() {
        if escaped {
            escaped = false;
        } else if quoted && character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if !quoted {
            for op in ["!=", ">=", "<=", "==", "=", ">", "<"] {
                if input[index..].starts_with(op) {
                    return Some((&input[..index], op, &input[index + op.len()..]));
                }
            }
        }
    }
    None
}

#[cfg(feature = "explore")]
fn is_field_lookup(field: &str) -> bool {
    let mut remaining = field.strip_prefix('.').unwrap_or("");
    if remaining.is_empty() {
        return false;
    }
    while !remaining.is_empty() {
        if let Some(bracket) = remaining.strip_prefix('[') {
            let mut quoted = false;
            let mut escaped = false;
            let end = bracket.char_indices().find_map(|(index, ch)| {
                if escaped {
                    escaped = false;
                } else if quoted && ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    quoted = !quoted;
                } else if ch == ']' && !quoted {
                    return Some(index);
                }
                None
            });
            let Some(end) = end else {
                return false;
            };
            let key = &bracket[..end];
            if !key.is_empty() && !is_json_scalar(key) {
                return false;
            }
            remaining = &bracket[end + 1..];
        } else {
            let end = remaining.find(['.', '[']).unwrap_or(remaining.len());
            if !is_identifier(&remaining[..end]) {
                return false;
            }
            remaining = &remaining[end..];
        }
        if let Some(next) = remaining.strip_prefix('.') {
            remaining = next;
        }
    }
    true
}

#[cfg(feature = "explore")]
fn is_json_scalar(value: &str) -> bool {
    let mut parsed = jaq_json::read::parse_many(value.as_bytes());
    let scalar = match parsed.next() {
        Some(Ok(jaq_json::Val::Num(_))) => is_number_literal(value),
        Some(Ok(jaq_json::Val::Null | jaq_json::Val::Bool(_) | jaq_json::Val::TStr(_))) => true,
        _ => false,
    };
    scalar && parsed.next().is_none()
}

fn is_number_literal(value: &str) -> bool {
    if !value.parse::<f64>().is_ok_and(f64::is_finite) {
        return false;
    }
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let mantissa = unsigned.split(['e', 'E']).next().unwrap_or("");
    let integer = mantissa.split('.').next().unwrap_or("");
    !integer.is_empty()
        && integer.bytes().all(|c| c.is_ascii_digit())
        && (integer == "0" || !integer.starts_with('0'))
        && !mantissa.ends_with('.')
}

#[cfg(feature = "explore")]
fn is_comparison_value(value: &str) -> bool {
    is_json_scalar(value)
        || (!value.is_empty()
            && !value.starts_with(['.', '"', '[', '{'])
            && !value.contains(['|', '(', ')', ';', '[', ']', '{', '}'])
            && !value.contains(" and ")
            && !value.contains(" or "))
}

fn where_condition(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("condition must not be empty".to_owned());
    }

    if let Some((field, op, value)) = split_condition(input) {
        let field = field.trim();
        let value = value.trim();
        if field.is_empty() || value.is_empty() {
            return Err(format!("expected FIELD{op}VALUE, got {input:?}"));
        }
        let jq_op = if op == "=" { "==" } else { op };
        return Ok(format!("{} {jq_op} {}", path_to_jq(field), literal(value)));
    }

    Err(format!(
        "expected a condition such as status=active, age>=18, or level!=DEBUG; got {input:?}"
    ))
}

fn path_to_jq(path: &str) -> String {
    let nested = nested_path_to_jq(path);
    if path.contains('.') {
        let key = quote(path);
        let direct = segment_to_jq(path);
        let exists = if is_number_literal(path) {
            format!("(has({key}) or has({path}))")
        } else {
            format!("has({key})")
        };
        format!("(if type == \"object\" and {exists} then {direct} else {nested} end)")
    } else {
        nested
    }
}

fn nested_path_to_jq(path: &str) -> String {
    let segments = path
        .split('.')
        .filter(|segment| !segment.is_empty())
        .map(segment_to_jq)
        .collect::<Vec<_>>();
    // Dynamic keys must be resolved against each intermediate value, not the
    // root input of a chained indexing expression.
    if segments.iter().any(|segment| segment.starts_with(".[(if ")) {
        segments.join(" | ")
    } else {
        segments.concat()
    }
}

fn segment_to_jq(segment: &str) -> String {
    if is_identifier(segment) {
        format!(".{segment}")
    } else {
        let key = quote(segment);
        // Prefer literal JSON string keys while retaining typed YAML keys and
        // numeric array indexing supported by the original shortcut syntax.
        if matches!(segment, "true" | "false" | "null") || is_number_literal(segment) {
            format!(".[(if type == \"object\" and has({key}) then {key} else {segment} end)]")
        } else {
            format!(".[{key}]")
        }
    }
}

fn is_identifier(segment: &str) -> bool {
    let mut chars = segment.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn literal(value: &str) -> String {
    match value {
        "true" | "false" | "null" => value.to_owned(),
        _ if is_number_literal(value) => value.to_owned(),
        _ if is_quoted(value) => value.to_owned(),
        _ => quote(value),
    }
}

fn is_quoted(value: &str) -> bool {
    value.len() >= 2 && value.starts_with('"') && value.ends_with('"')
}

fn quote(value: &str) -> String {
    // Keys are always strings, unlike typed comparison values.
    crate::output::format_json_value(&jaq_json::Val::utf8_str(value.to_owned()), false)
        .expect("a Rust string always serializes as UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_path_filters() {
        assert_eq!(path_filter("name").unwrap(), ".name");
        assert_eq!(
            path_filter("app.level").unwrap(),
            "(if type == \"object\" and has(\"app.level\") then .[\"app.level\"] else .app.level end)"
        );
        assert_eq!(
            path_filter("log.level").unwrap(),
            "(if type == \"object\" and has(\"log.level\") then .[\"log.level\"] else .log.level end)"
        );
        assert_eq!(path_filter("log-level").unwrap(), ".[\"log-level\"]");
    }

    #[test]
    fn numeric_boolean_and_control_keys_remain_strings() {
        for key in [
            "1.2",
            "1",
            "true",
            "null",
            "+1",
            "01",
            "1e309",
            "NaN",
            "line\nfeed",
            "quote\"key",
        ] {
            let input = jaq_json::Val::Obj(
                [(
                    jaq_json::Val::utf8_str(key.to_owned()),
                    jaq_json::Val::from(42isize),
                )]
                .into_iter()
                .collect::<jaq_json::Map<_, _>>()
                .into(),
            );
            let matcher = crate::matcher::Matcher::compile(&path_filter(key).unwrap()).unwrap();
            assert_eq!(
                matcher.apply(input).unwrap(),
                [jaq_json::Val::from(42isize)]
            );
        }
    }

    #[cfg(feature = "explore")]
    #[test]
    fn keeps_raw_output_expressions() {
        assert_eq!(
            output_expression_to_jq("{message, level: .log.level}").unwrap(),
            "{message, level: .log.level}"
        );
        assert_eq!(
            output_expression_to_jq("[.timestamp, .message]").unwrap(),
            "[.timestamp, .message]"
        );
        assert_eq!(output_expression_to_jq("message").unwrap(), ".message");
    }

    #[test]
    fn builds_where_filters() {
        assert_eq!(
            apply_where_filters(".name".to_owned(), &[String::from("active=true")]).unwrap(),
            "select(.active == true) | .name"
        );
        assert_eq!(
            apply_where_filters(
                ".".to_owned(),
                &[String::from("status=active"), String::from("age>=18")]
            )
            .unwrap(),
            "select(.status == \"active\" and .age >= 18) | ."
        );
        assert_eq!(
            apply_where_filters(".".to_owned(), &[String::from("log.level=ERROR")]).unwrap(),
            "select((if type == \"object\" and has(\"log.level\") then .[\"log.level\"] else .log.level end) == \"ERROR\") | ."
        );
    }

    #[cfg(feature = "explore")]
    #[test]
    fn expands_explore_expressions() {
        assert_eq!(expression_to_jq("").unwrap(), ".");
        assert_eq!(expression_to_jq("name").unwrap(), ".name");
        assert_eq!(expression_to_jq(".items[]").unwrap(), ".items[]");
        assert_eq!(
            expression_to_jq("status=active").unwrap(),
            "select(.status == \"active\") | ."
        );
    }

    #[test]
    fn comparisons_use_first_operator_and_accept_double_equals() {
        for (input, expected) in [
            ("level==ERROR", ".level == \"ERROR\""),
            ("message=age>=18", ".message == \"age>=18\""),
            ("message=\"a!=b\"", ".message == \"a!=b\""),
        ] {
            assert_eq!(where_condition(input).unwrap(), expected);
        }
    }

    #[cfg(feature = "explore")]
    #[test]
    fn completed_lookups_and_boolean_comparisons_select_records() {
        for expression in [".level=ERROR", ".level==ERROR", ".level == \"ERROR\""] {
            assert_eq!(
                expression_to_jq(expression).unwrap(),
                "select(.level == \"ERROR\") | ."
            );
        }
        assert_eq!(expression_to_jq(".[\"a=b\"]").unwrap(), ".[\"a=b\"]");
        assert_eq!(
            expression_to_jq(".[\"a=b\"]=ERROR").unwrap(),
            "select(.[\"a=b\"] == \"ERROR\") | ."
        );
        assert_eq!(
            expression_to_jq(".level | length > 3").unwrap(),
            ".level | length > 3"
        );
        assert_eq!(
            expression_to_jq(".count = .other").unwrap(),
            ".count = .other"
        );
        for value in ["ERROR-name", "+1", "NaN", "age>=18", "two words"] {
            assert_eq!(
                expression_to_jq(&format!(".message={value}")).unwrap(),
                format!("select(.message == {}) | .", quote(value))
            );
        }
    }
}
