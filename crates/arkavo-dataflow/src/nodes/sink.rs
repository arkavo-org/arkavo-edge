use super::NodeProcessor;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;

pub struct ConsoleSink;

#[async_trait]
impl NodeProcessor for ConsoleSink {
    async fn process(
        &self,
        input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        if let Some(data) = input {
            let format = params
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("json");

            match format {
                "json" => {
                    println!("{}", serde_json::to_string_pretty(&data)?);
                }
                "compact" => {
                    println!("{}", serde_json::to_string(&data)?);
                }
                "debug" => {
                    println!("{data:?}");
                }
                _ => {
                    println!("{data}");
                }
            }
        }

        Ok(None) // Sinks don't produce output
    }

    fn node_type(&self) -> &'static str {
        "sink"
    }
}

pub struct FileSink;

#[async_trait]
impl NodeProcessor for FileSink {
    async fn process(
        &self,
        input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        if let Some(data) = input {
            let path = params
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("path parameter required"))?;

            let append = params
                .get("append")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let format = params
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("jsonl");

            let content = match format {
                "json" => serde_json::to_string_pretty(&data)?,
                "jsonl" => format!("{}\n", serde_json::to_string(&data)?),
                "csv" => {
                    // Simple CSV conversion for flat objects
                    if let Value::Object(map) = &data {
                        map.values()
                            .map(|v| match v {
                                Value::String(s) => format!("\"{}\"", s.replace('"', "\"\"")),
                                v => v.to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join(",")
                            + "\n"
                    } else {
                        serde_json::to_string(&data)?
                    }
                }
                _ => data.to_string(),
            };

            if append {
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)?;
                file.write_all(content.as_bytes())?;
            } else {
                std::fs::write(path, content)?;
            }
        }

        Ok(None)
    }

    fn node_type(&self) -> &'static str {
        "sink"
    }
}

pub struct TaskWindowSink;

#[async_trait]
impl NodeProcessor for TaskWindowSink {
    async fn process(
        &self,
        input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        if let Some(data) = input {
            let task_id = params
                .get("task_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("task_id parameter required"))?;

            // In a real implementation, this would send to the terminal UI task window
            // For now, just log it
            eprintln!("[TaskWindow {}] {}", task_id, serde_json::to_string(&data)?);
        }

        Ok(None)
    }

    fn node_type(&self) -> &'static str {
        "sink"
    }
}

pub struct MetricsSink;

#[async_trait]
impl NodeProcessor for MetricsSink {
    async fn process(
        &self,
        input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        if let Some(data) = input {
            let metric_name = params
                .get("metric_name")
                .and_then(|v| v.as_str())
                .unwrap_or("dataflow_metric");

            let metric_type = params
                .get("metric_type")
                .and_then(|v| v.as_str())
                .unwrap_or("counter");

            // Extract value from data
            let value = data.get("value").and_then(|v| v.as_f64()).unwrap_or(1.0);

            // In a real implementation, this would send to a metrics system
            eprintln!("[Metric] {metric_name} ({metric_type}) = {value}");
        }

        Ok(None)
    }

    fn node_type(&self) -> &'static str {
        "sink"
    }
}

pub struct WebhookSink;

#[async_trait]
impl NodeProcessor for WebhookSink {
    async fn process(
        &self,
        input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        if let Some(data) = input {
            let url = params
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("url parameter required"))?;

            let method = params
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("POST");

            // In a real implementation, this would make an HTTP request
            // For now, just log it
            eprintln!(
                "[Webhook] {} {} - {}",
                method,
                url,
                serde_json::to_string(&data)?
            );
        }

        Ok(None)
    }

    fn node_type(&self) -> &'static str {
        "sink"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_console_sink() {
        let sink = ConsoleSink;
        let mut params = HashMap::new();
        params.insert("format".to_string(), serde_json::json!("compact"));

        let input = serde_json::json!({"test": "data"});
        let result = sink.process(Some(input), &params).await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_none()); // Sinks return None
    }

    #[tokio::test]
    async fn test_file_sink() {
        let sink = FileSink;
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let mut params = HashMap::new();
        params.insert(
            "path".to_string(),
            serde_json::json!(file_path.to_str().unwrap()),
        );
        params.insert("format".to_string(), serde_json::json!("jsonl"));

        let input = serde_json::json!({"test": "data"});
        let result = sink.process(Some(input), &params).await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());

        // Verify file was written
        let contents = std::fs::read_to_string(&file_path).unwrap();
        assert!(contents.contains("\"test\":\"data\""));
    }
}
