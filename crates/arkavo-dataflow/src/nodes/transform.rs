use super::NodeProcessor;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

pub struct JsonTransform;

#[async_trait]
impl NodeProcessor for JsonTransform {
    async fn process(
        &self,
        input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        let Some(data) = input else {
            return Ok(None);
        };

        // Get transformation spec from params
        let transform_spec = params
            .get("spec")
            .ok_or_else(|| anyhow::anyhow!("spec parameter required"))?;

        // Apply transformation based on spec
        match transform_spec {
            Value::Object(spec) => {
                let mut result = json!({});
                let result_obj = result.as_object_mut().unwrap();

                for (key, mapping) in spec {
                    if let Some(source_path) = mapping.as_str() {
                        if let Some(value) = extract_path(&data, source_path) {
                            result_obj.insert(key.clone(), value.clone());
                        }
                    }
                }

                Ok(Some(result))
            }
            _ => Err(anyhow::anyhow!("Invalid transform spec")),
        }
    }

    fn node_type(&self) -> &'static str {
        "transform"
    }
}

pub struct FilterTransform;

#[async_trait]
impl NodeProcessor for FilterTransform {
    async fn process(
        &self,
        input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        let Some(data) = input else {
            return Ok(None);
        };

        // Get filter conditions from params
        if let Some(conditions) = params.get("conditions").and_then(|v| v.as_array()) {
            for condition in conditions {
                if !evaluate_condition(&data, condition) {
                    return Ok(None); // Filter out this message
                }
            }
        }

        Ok(Some(data))
    }

    fn node_type(&self) -> &'static str {
        "transform"
    }
}

pub struct EnrichTransform;

#[async_trait]
impl NodeProcessor for EnrichTransform {
    async fn process(
        &self,
        input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        let Some(mut data) = input else {
            return Ok(None);
        };

        // Add enrichment fields
        if let Some(fields) = params.get("fields").and_then(|v| v.as_object()) {
            if let Value::Object(ref mut data_obj) = data {
                for (key, value) in fields {
                    data_obj.insert(key.clone(), value.clone());
                }
            }
        }

        // Add metadata
        if params
            .get("add_metadata")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            if let Value::Object(ref mut data_obj) = data {
                data_obj.insert(
                    "_enriched_at".to_string(),
                    json!(chrono::Utc::now().to_rfc3339()),
                );
                data_obj.insert("_enricher".to_string(), json!("enrich_transform"));
            }
        }

        Ok(Some(data))
    }

    fn node_type(&self) -> &'static str {
        "transform"
    }
}

pub struct AggregateTransform;

#[async_trait]
impl NodeProcessor for AggregateTransform {
    async fn process(
        &self,
        input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        let Some(data) = input else {
            return Ok(None);
        };

        // Get aggregation config
        let window_size = params
            .get("window_size")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(10);

        let aggregation_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("count");

        // In a real implementation, this would maintain state across multiple messages
        // For now, just return a simple aggregation result
        Ok(Some(json!({
            "aggregation_type": aggregation_type,
            "window_size": window_size,
            "value": match aggregation_type {
                "count" => json!(1),
                "sum" => data.get("value").cloned().unwrap_or_else(|| json!(0)),
                "avg" => data.get("value").cloned().unwrap_or_else(|| json!(0)),
                _ => json!(null),
            },
            "timestamp": chrono::Utc::now().to_rfc3339(),
        })))
    }

    fn node_type(&self) -> &'static str {
        "transform"
    }
}

fn extract_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;

    for part in parts {
        match current {
            Value::Object(map) => {
                current = map.get(part)?;
            }
            Value::Array(arr) => {
                let index: usize = part.parse().ok()?;
                current = arr.get(index)?;
            }
            _ => return None,
        }
    }

    Some(current)
}

fn evaluate_condition(data: &Value, condition: &Value) -> bool {
    if let Some(cond_obj) = condition.as_object() {
        let field = cond_obj.get("field").and_then(|v| v.as_str()).unwrap_or("");
        let operator = cond_obj
            .get("operator")
            .and_then(|v| v.as_str())
            .unwrap_or("equals");
        let value = cond_obj.get("value");

        if let (Some(field_value), Some(compare_value)) = (extract_path(data, field), value) {
            match operator {
                "equals" => field_value == compare_value,
                "contains" => {
                    if let (Value::String(haystack), Value::String(needle)) =
                        (field_value, compare_value)
                    {
                        haystack.contains(needle)
                    } else {
                        false
                    }
                }
                "gt" => {
                    if let (Some(a), Some(b)) = (field_value.as_f64(), compare_value.as_f64()) {
                        a > b
                    } else {
                        false
                    }
                }
                "lt" => {
                    if let (Some(a), Some(b)) = (field_value.as_f64(), compare_value.as_f64()) {
                        a < b
                    } else {
                        false
                    }
                }
                _ => true,
            }
        } else {
            false
        }
    } else {
        true
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::collection_is_never_read)]
#[allow(clippy::needless_collect)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_json_transform() {
        let transform = JsonTransform;
        let mut params = HashMap::new();
        params.insert(
            "spec".to_string(),
            json!({
                "name": "user.name",
                "email": "user.email",
                "age": "user.details.age"
            }),
        );

        let input = json!({
            "user": {
                "name": "John Doe",
                "email": "john@example.com",
                "details": {
                    "age": 30
                }
            }
        });

        let result = transform.process(Some(input), &params).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        assert_eq!(output["name"], "John Doe");
        assert_eq!(output["email"], "john@example.com");
        assert_eq!(output["age"], 30);
    }

    #[tokio::test]
    async fn test_filter_transform() {
        let transform = FilterTransform;
        let mut params = HashMap::new();
        params.insert(
            "conditions".to_string(),
            json!([
                {
                    "field": "level",
                    "operator": "equals",
                    "value": "error"
                }
            ]),
        );

        // Should pass
        let input1 = json!({"level": "error", "message": "test"});
        let result1 = transform.process(Some(input1), &params).await.unwrap();
        assert!(result1.is_some());

        // Should be filtered out
        let input2 = json!({"level": "info", "message": "test"});
        let result2 = transform.process(Some(input2), &params).await.unwrap();
        assert!(result2.is_none());
    }
}
