use serde_json::{Value, json};

/// Responses strict schemas require every property. Optional source properties
/// become nullable so the model can still express their absence faithfully.
pub(super) fn strict(mut schema: Value) -> Value {
    normalize(&mut schema);
    schema
}

fn normalize(schema: &mut Value) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        let required = object
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut properties = properties.clone();
        for (name, property) in &mut properties {
            normalize(property);
            if !required.iter().any(|v| v.as_str() == Some(name)) {
                *property = json!({"anyOf":[property.clone(), {"type":"null"}]});
            }
        }
        object.insert(
            "required".into(),
            Value::Array(properties.keys().map(|k| json!(k)).collect()),
        );
        object.insert("properties".into(), Value::Object(properties));
        object.insert("additionalProperties".into(), Value::Bool(false));
    } else if object.get("type").and_then(Value::as_str) == Some("object") {
        object.insert("properties".into(), json!({}));
        object.insert("required".into(), json!([]));
        object.insert("additionalProperties".into(), Value::Bool(false));
    }
    for key in ["items", "additionalProperties"] {
        if let Some(value) = object.get_mut(key) {
            normalize(value);
        }
    }
    for key in ["anyOf", "oneOf", "allOf", "prefixItems"] {
        if let Some(values) = object.get_mut(key).and_then(Value::as_array_mut) {
            for value in values {
                normalize(value);
            }
        }
    }
    for key in ["$defs", "definitions"] {
        if let Some(values) = object.get_mut(key).and_then(Value::as_object_mut) {
            for value in values.values_mut() {
                normalize(value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[arkavo_test_macros::spec("ASTRA-001")]
    #[test]
    fn nested_optional_properties_become_nullable() {
        let schema = strict(
            json!({"type":"object","properties":{"tasks":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"note":{"type":"string"}},"required":["name"]}}},"required":["tasks"]}),
        );
        assert_eq!(schema["additionalProperties"], false);
        let item = &schema["properties"]["tasks"]["items"];
        assert_eq!(item["required"], json!(["name", "note"]));
        assert_eq!(item["properties"]["name"]["type"], "string");
        assert_eq!(item["properties"]["note"]["anyOf"][1]["type"], "null");
    }
}
