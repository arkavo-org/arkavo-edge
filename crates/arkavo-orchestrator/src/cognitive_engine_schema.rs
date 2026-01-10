//! JSON schema definitions for structured LLM outputs in planning.
//!
//! Provides serde-compatible structs that mirror ExecutionPlan with JSON Schema
//! annotations for providers that support structured outputs (e.g., Gemini).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// JSON-serializable execution plan for structured LLM output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonExecutionPlan {
    pub steps: Vec<JsonPlanStep>,
}

/// JSON-serializable plan step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonPlanStep {
    pub step_number: usize,
    pub description: String,
    pub commands: Vec<String>,
    /// Verification types: "tests", "linter", "build", "file_constraint_400"
    pub verification: Vec<String>,
    pub confidence: f32,
}

impl JsonExecutionPlan {
    /// Generate JSON Schema for Gemini structured output
    pub fn json_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "steps": {
                    "type": "array",
                    "description": "List of execution steps for the plan",
                    "items": {
                        "type": "object",
                        "properties": {
                            "step_number": {
                                "type": "integer",
                                "description": "Sequential step number starting from 1"
                            },
                            "description": {
                                "type": "string",
                                "description": "Brief description of what this step does"
                            },
                            "commands": {
                                "type": "array",
                                "description": "Shell commands to execute for this step",
                                "items": {"type": "string"}
                            },
                            "verification": {
                                "type": "array",
                                "description": "Verification checks: tests, linter, build, or file_constraint_400",
                                "items": {"type": "string"}
                            },
                            "confidence": {
                                "type": "number",
                                "description": "Confidence score between 0.0 and 1.0",
                                "minimum": 0.0,
                                "maximum": 1.0
                            }
                        },
                        "required": ["step_number", "description", "commands", "verification", "confidence"]
                    }
                }
            },
            "required": ["steps"]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_schema_generation() {
        let schema = JsonExecutionPlan::json_schema();
        assert!(schema.get("properties").is_some());
        assert!(schema.get("required").is_some());

        let props = schema.get("properties").unwrap();
        assert!(props.get("steps").is_some());
    }

    #[test]
    fn test_deserialize_json_plan() {
        let json = r#"{
            "steps": [
                {
                    "step_number": 1,
                    "description": "Run tests",
                    "commands": ["cargo test"],
                    "verification": ["tests", "build"],
                    "confidence": 0.9
                },
                {
                    "step_number": 2,
                    "description": "Check linter",
                    "commands": ["cargo clippy"],
                    "verification": ["linter"],
                    "confidence": 0.85
                }
            ]
        }"#;

        let plan: JsonExecutionPlan = serde_json::from_str(json).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].step_number, 1);
        assert_eq!(plan.steps[0].description, "Run tests");
        assert_eq!(plan.steps[0].commands, vec!["cargo test"]);
        assert_eq!(plan.steps[0].verification, vec!["tests", "build"]);
        assert!((plan.steps[0].confidence - 0.9).abs() < 0.01);
    }

    #[test]
    fn test_serialize_json_plan() {
        let plan = JsonExecutionPlan {
            steps: vec![JsonPlanStep {
                step_number: 1,
                description: "Build project".to_string(),
                commands: vec!["cargo build".to_string()],
                verification: vec!["build".to_string()],
                confidence: 0.95,
            }],
        };

        let json = serde_json::to_string(&plan).unwrap();
        assert!(json.contains("Build project"));
        assert!(json.contains("cargo build"));
    }

    #[test]
    fn test_empty_plan() {
        let json = r#"{"steps": []}"#;
        let plan: JsonExecutionPlan = serde_json::from_str(json).unwrap();
        assert!(plan.steps.is_empty());
    }
}
