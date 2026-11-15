//! Checks a tool's arguments against the input schema it advertises, at
//! every level, before anything reaches the service: an argument the schema
//! does not name (such as a `source` inside one Fact) is refused, never
//! passed through.
//!
//! Only the JSON-schema keywords the catalogue uses are understood: `type`,
//! `properties`, `required`, `additionalProperties: false`, `items`,
//! `minItems`, `enum`, `minimum`, `maximum` and the one `pattern`, the
//! source's. A schema with none of them (`{"description": ...}`) takes any
//! value.

use serde_json::Value;

/// `Err` names the first argument that does not fit, by its path
/// (`facts[0].source`), and why.
pub fn check(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    if let Some(kind) = schema.get("type").and_then(Value::as_str) {
        let fits = match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            _ => true,
        };
        if !fits {
            return Err(format!("{} must be {kind}", name(path)));
        }
    }
    if let Some(choices) = schema.get("enum").and_then(Value::as_array) {
        if !choices.contains(value) {
            return Err(format!(
                "{} must be one of {}",
                name(path),
                Value::from(choices.clone())
            ));
        }
    }
    if let Some(number) = value.as_f64() {
        if schema
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|min| number < min)
        {
            return Err(format!("{} is below its minimum", name(path)));
        }
        if schema
            .get("maximum")
            .and_then(Value::as_f64)
            .is_some_and(|max| number > max)
        {
            return Err(format!("{} is above its maximum", name(path)));
        }
    }
    if let (Some(pattern), Some(text)) = (
        schema.get("pattern").and_then(Value::as_str),
        value.as_str(),
    ) {
        if !matches_pattern(pattern, text) {
            return Err(format!("{} must match {pattern}", name(path)));
        }
    }
    if let Some(items) = value.as_array() {
        if schema
            .get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|min| (items.len() as u64) < min)
        {
            return Err(format!("{} needs more items", name(path)));
        }
        if let Some(item) = schema.get("items") {
            for (index, one) in items.iter().enumerate() {
                check(item, one, &format!("{path}[{index}]"))?;
            }
        }
    }
    if let Some(object) = value.as_object() {
        let properties = schema.get("properties").and_then(Value::as_object);
        let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
        for (key, one) in object {
            let inner = join(path, key);
            match properties.and_then(|known| known.get(key)) {
                Some(property) => check(property, one, &inner)?,
                None if closed => {
                    return Err(format!("{inner} is not an argument this tool takes"))
                }
                None => {}
            }
        }
        for key in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(key) = key.as_str() {
                if !object.contains_key(key) {
                    return Err(format!("{} is required", join(path, key)));
                }
            }
        }
    }
    Ok(())
}

/// The catalogue's one pattern, the source's, without a regex engine.
fn matches_pattern(pattern: &str, text: &str) -> bool {
    match pattern {
        "^(agent|tool|web):.+$" => text.split_once(':').is_some_and(|(kind, rest)| {
            matches!(kind, "agent" | "tool" | "web") && !rest.is_empty()
        }),
        _ => true,
    }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn name(path: &str) -> &str {
    if path.is_empty() {
        "the arguments"
    } else {
        path
    }
}
