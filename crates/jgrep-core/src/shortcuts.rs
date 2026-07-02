pub fn path_filter(path: &str) -> Result<String, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("path must not be empty".to_owned());
    }
    Ok(path_to_jq(path))
}

pub fn apply_where_filters(base_filter: String, filters: &[String]) -> Result<String, String> {
    if filters.is_empty() {
        return Ok(base_filter);
    }

    let mut conditions = Vec::with_capacity(filters.len());
    for filter in filters {
        conditions.push(where_condition(filter)?);
    }

    Ok(format!(
        "select({}) | {base_filter}",
        conditions.join(" and ")
    ))
}

fn where_condition(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("condition must not be empty".to_owned());
    }

    for op in ["!=", ">=", "<=", "=", ">", "<"] {
        if let Some((field, value)) = input.split_once(op) {
            let field = field.trim();
            let value = value.trim();
            if field.is_empty() || value.is_empty() {
                return Err(format!("expected FIELD{op}VALUE, got {input:?}"));
            }
            let jq_op = if op == "=" { "==" } else { op };
            return Ok(format!("{} {jq_op} {}", path_to_jq(field), literal(value)));
        }
    }

    Err(format!(
        "expected a condition such as status=active, age>=18, or level!=DEBUG; got {input:?}"
    ))
}

fn path_to_jq(path: &str) -> String {
    let nested = nested_path_to_jq(path);
    if path.contains('.') {
        format!(
            "(if type == \"object\" and has({}) then .[{}] else {nested} end)",
            literal(path),
            literal(path)
        )
    } else {
        nested
    }
}

fn nested_path_to_jq(path: &str) -> String {
    path.split('.')
        .filter(|segment| !segment.is_empty())
        .map(segment_to_jq)
        .collect()
}

fn segment_to_jq(segment: &str) -> String {
    if is_identifier(segment) {
        format!(".{segment}")
    } else {
        format!(".[{}]", literal(segment))
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
        _ if value.parse::<f64>().is_ok() => value.to_owned(),
        _ if is_quoted(value) => value.to_owned(),
        _ => quote(value),
    }
}

fn is_quoted(value: &str) -> bool {
    value.len() >= 2 && value.starts_with('"') && value.ends_with('"')
}

fn quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
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
}
