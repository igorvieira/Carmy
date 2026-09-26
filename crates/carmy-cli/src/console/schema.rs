//! Reads just enough JSON Schema to prefill arguments and summarize fields.
use serde_json::{Map, Value, json};

fn resolve<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix("#/"))
        .and_then(|path| path.split('/').try_fold(root, |node, key| node.get(key)))
        .unwrap_or(schema)
}

fn type_of(schema: &Value) -> Option<&str> {
    match schema.get("type") {
        Some(Value::String(t)) => Some(t.as_str()),
        // `["string", "null"]` for an `Option<String>`: the non-null type.
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null"),
        _ => None,
    }
}

fn sample(schema: &Value, root: &Value, depth: usize) -> Value {
    let schema = resolve(schema, root);
    if let Some(value) = schema.get("default").or_else(|| schema.get("const")) {
        return value.clone();
    }
    if let Some(first) = schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|e| e.first())
    {
        return first.clone();
    }
    match type_of(schema) {
        Some("string") => json!(""),
        Some("integer") | Some("number") => json!(0),
        Some("boolean") => json!(false),
        Some("array") => json!([]),
        Some("null") => Value::Null,
        _ if depth > 3 => json!({}),
        _ => {
            let mut object = Map::new();
            if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                for (name, property) in properties {
                    object.insert(name.clone(), sample(property, root, depth + 1));
                }
            }
            Value::Object(object)
        }
    }
}

/// Example arguments for an input schema, as one line of JSON.
pub fn skeleton(schema: &Value) -> String {
    sample(schema, schema, 0).to_string()
}

/// `name: type` for each property, `?` marking optional ones; the type alone otherwise.
pub fn fields(schema: &Value) -> String {
    let root = schema;
    let schema = resolve(schema, root);
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return type_of(schema).unwrap_or("any").to_owned();
    };
    if properties.is_empty() {
        return "{}".into();
    }
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let parts: Vec<String> = properties
        .iter()
        .map(|(name, property)| {
            let property = resolve(property, root);
            let kind = match type_of(property) {
                Some("array") => {
                    let item = property.get("items").map(|i| resolve(i, root));
                    format!("[{}]", item.and_then(type_of).unwrap_or("object"))
                }
                Some(t) => t.to_owned(),
                None => "object".to_owned(),
            };
            let optional = if required.contains(&name.as_str()) {
                ""
            } else {
                "?"
            };
            format!("{name}{optional}: {kind}")
        })
        .collect();
    format!("{{ {} }}", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skeletons_follow_the_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "sku": {"type": "string"},
                "quantity": {"type": "integer"},
                "gift": {"type": ["boolean", "null"]},
                "address": {"$ref": "#/$defs/Address"},
            },
            "required": ["sku", "quantity"],
            "$defs": {"Address": {"type": "object", "properties": {"city": {"type": "string"}}}},
        });
        let value: Value = serde_json::from_str(&skeleton(&schema)).unwrap();
        assert_eq!(
            value,
            json!({"sku": "", "quantity": 0, "gift": false, "address": {"city": ""}})
        );
        assert_eq!(
            fields(&schema),
            "{ address?: object, gift?: boolean, quantity: integer, sku: string }"
        );
        assert_eq!(skeleton(&json!({"type": "string"})), "\"\"");
        assert_eq!(fields(&json!({"type": "object", "properties": {}})), "{}");
    }
}
