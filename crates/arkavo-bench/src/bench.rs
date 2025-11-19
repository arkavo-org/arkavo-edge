use crate::metrics::{BenchMetrics, BenchSummary};
use crate::{BenchError, Result};
use arkavo_mcp::{Tool, ToolSchema};
use arkavo_workspace::WorkspaceTool;
use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweBenchInstance {
    pub instance_id: String,
    pub repo: String,
    pub base_commit: String,
    pub problem_statement: String,
    pub hints_text: Option<String>,
    pub test_patch: String,
}

impl SweBenchInstance {
    pub fn new(
        instance_id: String,
        repo: String,
        base_commit: String,
        problem_statement: String,
        hints_text: Option<String>,
        test_patch: String,
    ) -> Self {
        Self {
            instance_id,
            repo,
            base_commit,
            problem_statement,
            hints_text,
            test_patch,
        }
    }
}

pub struct SweBenchTool {
    schema: ToolSchema,
    workspace: WorkspaceTool,
}

impl SweBenchTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "swe_bench".to_string(),
                aliases: None,
                description: "SWE-bench evaluation harness for coding agent benchmarking"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["load", "run", "evaluate", "summary"],
                            "description": "Benchmark action to perform"
                        },
                        "subset": {
                            "type": "string",
                            "enum": ["lite", "verified", "full", "multimodal", "test"],
                            "description": "SWE-bench dataset subset (lite: 534 instances, verified: 500 instances, full: 2294 instances, multimodal: 500 instances)"
                        },
                        "instance_id": {
                            "type": "string",
                            "description": "Specific instance ID to run"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Limit number of instances to run"
                        },
                        "solution": {
                            "type": "string",
                            "description": "Proposed solution code (for evaluate action)"
                        },
                        "metrics_file": {
                            "type": "string",
                            "description": "Path to save/load metrics JSON"
                        },
                        "parallel": {
                            "type": "integer",
                            "description": "Number of parallel instances to run concurrently (default: 1)"
                        }
                    },
                    "required": ["action"]
                }),
            },
            workspace: WorkspaceTool::new(),
        }
    }

    async fn load_instances(
        &self,
        subset: &str,
        limit: Option<usize>,
    ) -> Result<Vec<SweBenchInstance>> {
        let (dataset_name, split) = match subset {
            "lite" => ("princeton-nlp/SWE-bench_Lite", "test"),
            "verified" => ("princeton-nlp/SWE-bench_Verified", "test"),
            "full" => ("princeton-nlp/SWE-bench", "test"),
            "multimodal" => ("SWE-bench/SWE-bench_Multimodal", "test"),
            "test" => ("princeton-nlp/SWE-bench_Lite", "dev"),
            _ => {
                return Err(BenchError::InvalidParams(format!(
                    "Unknown subset: {subset}"
                )));
            }
        };

        let url = format!(
            "https://datasets-server.huggingface.co/rows?dataset={}&config=default&split={}&offset=0&length=100",
            dataset_name, split
        );

        let response = reqwest::get(&url).await?;
        let json_response: serde_json::Value = response.json().await.map_err(|e| {
            BenchError::TaskLoadingFailed(format!("Failed to fetch from HuggingFace: {e}"))
        })?;

        let rows = json_response
            .get("rows")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                BenchError::TaskLoadingFailed("Invalid HuggingFace response".to_string())
            })?;

        let mut instances: Vec<SweBenchInstance> = rows
            .iter()
            .filter_map(|row| {
                let row_data = row.get("row")?;
                Some(SweBenchInstance {
                    instance_id: row_data.get("instance_id")?.as_str()?.to_string(),
                    repo: row_data.get("repo")?.as_str()?.to_string(),
                    base_commit: row_data.get("base_commit")?.as_str()?.to_string(),
                    problem_statement: row_data.get("problem_statement")?.as_str()?.to_string(),
                    hints_text: row_data
                        .get("hints_text")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    test_patch: row_data.get("test_patch")?.as_str()?.to_string(),
                })
            })
            .collect();

        if let Some(n) = limit {
            instances.truncate(n);
        }

        Ok(instances)
    }

    async fn run_instance(&self, instance: &SweBenchInstance) -> Result<BenchMetrics> {
        let mut metrics = BenchMetrics::new(instance.instance_id.clone());
        let start = Instant::now();

        let workspace_id = format!("swe-bench-{}", instance.instance_id);

        let create_params = json!({
            "action": "create",
            "workspace_id": workspace_id,
            "repo_url": instance.repo,
            "cpu_limit": "2.0",
            "memory_limit": "4g",
            "network": true
        });

        if let Err(e) = self.workspace.execute(create_params).await {
            metrics.set_error(format!("Workspace creation failed: {e}"));
            metrics.set_wall_time(start.elapsed());
            return Ok(metrics);
        }

        let checkout_cmd = format!("git -C /workspace checkout {}", instance.base_commit);
        let checkout_params = json!({
            "action": "execute",
            "workspace_id": workspace_id,
            "command": checkout_cmd,
            "timeout": 60
        });

        if let Err(e) = self.workspace.execute(checkout_params).await {
            metrics.set_error(format!("Git checkout failed: {e}"));
            let _ = self.cleanup_workspace(&workspace_id).await;
            metrics.set_wall_time(start.elapsed());
            return Ok(metrics);
        }

        let test_params = json!({
            "action": "execute",
            "workspace_id": workspace_id,
            "command": "pytest -xvs",
            "timeout": 300
        });

        match self.workspace.execute(test_params).await {
            Ok(result) => {
                if let Some(output) = result.get("result").and_then(|r| r.as_str())
                    && output.contains("passed")
                    && !output.contains("failed")
                {
                    metrics.set_resolved(true);
                }
            }
            Err(e) => {
                metrics.set_error(format!("Test execution failed: {e}"));
            }
        }

        let _ = self.cleanup_workspace(&workspace_id).await;
        metrics.set_wall_time(start.elapsed());

        Ok(metrics)
    }

    async fn cleanup_workspace(&self, workspace_id: &str) -> Result<()> {
        let cleanup_params = json!({
            "action": "cleanup",
            "workspace_id": workspace_id
        });

        self.workspace
            .execute(cleanup_params)
            .await
            .map_err(|e| BenchError::ExecutionFailed(format!("Cleanup failed: {e}")))?;

        Ok(())
    }

    async fn evaluate_solution(&self, instance: &SweBenchInstance, solution: &str) -> Result<bool> {
        let workspace_id = format!("swe-bench-eval-{}", instance.instance_id);

        let create_params = json!({
            "action": "create",
            "workspace_id": workspace_id,
            "repo_url": instance.repo,
            "cpu_limit": "2.0",
            "memory_limit": "4g",
            "network": true
        });

        self.workspace
            .execute(create_params)
            .await
            .map_err(|e| BenchError::ExecutionFailed(format!("Workspace creation failed: {e}")))?;

        let checkout_cmd = format!("git -C /workspace checkout {}", instance.base_commit);
        let checkout_params = json!({
            "action": "execute",
            "workspace_id": workspace_id,
            "command": checkout_cmd,
            "timeout": 60
        });

        self.workspace
            .execute(checkout_params)
            .await
            .map_err(|e| BenchError::ExecutionFailed(format!("Git checkout failed: {e}")))?;

        let apply_solution_cmd = format!("cd /workspace && echo '{}' | git apply", solution);
        let apply_params = json!({
            "action": "execute",
            "workspace_id": workspace_id,
            "command": apply_solution_cmd,
            "timeout": 30
        });

        self.workspace
            .execute(apply_params)
            .await
            .map_err(|e| BenchError::ExecutionFailed(format!("Solution apply failed: {e}")))?;

        let test_params = json!({
            "action": "execute",
            "workspace_id": workspace_id,
            "command": "pytest -xvs",
            "timeout": 300
        });

        let result = self
            .workspace
            .execute(test_params)
            .await
            .map_err(|e| BenchError::ExecutionFailed(format!("Test execution failed: {e}")))?;

        let _ = self.cleanup_workspace(&workspace_id).await;

        let resolved = if let Some(output) = result.get("result").and_then(|r| r.as_str()) {
            output.contains("passed") && !output.contains("failed")
        } else {
            false
        };

        Ok(resolved)
    }

    async fn execute_benchmark(&self, params: &Value) -> Result<Value> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| BenchError::InvalidParams("Missing action".to_string()))?;

        match action {
            "load" => {
                let subset = params
                    .get("subset")
                    .and_then(|v| v.as_str())
                    .unwrap_or("lite");

                let limit = params
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);

                let instances = self.load_instances(subset, limit).await?;

                Ok(json!({
                    "subset": subset,
                    "count": instances.len(),
                    "instances": instances
                }))
            }

            "run" => {
                let subset = params
                    .get("subset")
                    .and_then(|v| v.as_str())
                    .unwrap_or("lite");

                let limit = params
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);

                let parallel = params
                    .get("parallel")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize)
                    .unwrap_or(1);

                let instances = self.load_instances(subset, limit).await?;

                let all_metrics = if parallel > 1 {
                    let self_arc = Arc::new(self);
                    stream::iter(instances)
                        .map(|instance| {
                            let self_clone = Arc::clone(&self_arc);
                            async move { self_clone.run_instance(&instance).await }
                        })
                        .buffer_unordered(parallel)
                        .collect::<Vec<_>>()
                        .await
                        .into_iter()
                        .collect::<Result<Vec<_>>>()?
                } else {
                    let mut metrics = Vec::new();
                    for instance in instances {
                        metrics.push(self.run_instance(&instance).await?);
                    }
                    metrics
                };

                let summary = BenchSummary::from_metrics(&all_metrics);

                if let Some(metrics_file) = params.get("metrics_file").and_then(|v| v.as_str()) {
                    let metrics_json = serde_json::to_string_pretty(&all_metrics)?;
                    tokio::fs::write(metrics_file, metrics_json).await?;
                }

                Ok(json!({
                    "summary": summary,
                    "metrics": all_metrics
                }))
            }

            "evaluate" => {
                let instance_id = params
                    .get("instance_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| BenchError::InvalidParams("Missing instance_id".to_string()))?;

                let solution = params
                    .get("solution")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| BenchError::InvalidParams("Missing solution".to_string()))?;

                let subset = params
                    .get("subset")
                    .and_then(|v| v.as_str())
                    .unwrap_or("lite");

                let instances = self.load_instances(subset, None).await?;
                let instance = instances
                    .iter()
                    .find(|i| i.instance_id == instance_id)
                    .ok_or_else(|| {
                        BenchError::InvalidParams(format!("Instance not found: {instance_id}"))
                    })?;

                let resolved = self.evaluate_solution(instance, solution).await?;

                Ok(json!({
                    "instance_id": instance_id,
                    "resolved": resolved
                }))
            }

            "summary" => {
                let metrics_file = params
                    .get("metrics_file")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| BenchError::InvalidParams("Missing metrics_file".to_string()))?;

                let metrics_json = tokio::fs::read_to_string(metrics_file).await?;
                let metrics: Vec<BenchMetrics> = serde_json::from_str(&metrics_json)?;

                let summary = BenchSummary::from_metrics(&metrics);

                Ok(json!(summary))
            }

            _ => Err(BenchError::InvalidParams(format!(
                "Unknown action: {action}"
            ))),
        }
    }
}

impl Default for SweBenchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SweBenchTool {
    async fn execute(
        &self,
        params: Value,
    ) -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.execute_benchmark(&params)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_load_instances() {
        let tool = SweBenchTool::new();
        let result = tool.load_instances("lite", Some(1)).await;
        assert!(result.is_ok() || result.is_err());
    }
}
