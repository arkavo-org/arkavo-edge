use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Result, template};

/// The host must recheck grants and data policy for every invocation, and
/// observe each result before allowing another call. Denials are errors.
#[async_trait]
pub trait Executor: Send + Sync {
    fn available(&self, tool: &str) -> bool;
    async fn execute(&self, tool: &str, arguments: Value) -> Result<Value>;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub pointer: String,
    pub equals: Value,
}

impl Check {
    pub(crate) fn matches(&self, result: &Value, inputs: &Value) -> Result<bool> {
        Ok(result.pointer(&self.pointer) == Some(&template::bind(&self.equals, inputs)?))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub tool: String,
    pub arguments: Value,
    pub check: Check,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Routine {
    pub steps: Vec<Step>,
}

impl Routine {
    pub fn validate(&self) -> Result<()> {
        if !(2..=8).contains(&self.steps.len()) {
            return Err("A routine requires 2 to 8 steps".into());
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 16_384 {
            return Err("Routine exceeds 16 KiB".into());
        }
        for step in &self.steps {
            if step.tool.is_empty()
                || step.tool.len() > 128
                || step.tool.starts_with("routine_")
                || !step
                    .tool
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-.:/".contains(c))
            {
                return Err("Invalid or recursive tool name".into());
            }
            if !step.arguments.is_object() {
                return Err("Tool arguments must be an object".into());
            }
            if !step.check.pointer.starts_with('/') || step.check.pointer.len() > 256 {
                return Err("Each step requires a bounded result JSON pointer".into());
            }
            template::validate(&step.arguments)?;
            template::validate(&step.check.equals)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid() -> Routine {
        Routine {
            steps: (0..2)
                .map(|_| Step {
                    tool: "browser.read".into(),
                    arguments: json!({"selector":{"$input":"selector"}}),
                    check: Check {
                        pointer: "/ok".into(),
                        equals: json!(true),
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn rejects_unbounded_recursive_and_unchecked_programs() {
        assert!(valid().validate().is_ok());
        assert!(Routine { steps: vec![] }.validate().is_err());
        let mut r = valid();
        r.steps = vec![r.steps[0].clone(); 9];
        assert!(r.validate().is_err());
        for tool in ["", "routine_run", "shell; command"] {
            let mut r = valid();
            r.steps[0].tool = tool.into();
            assert!(r.validate().is_err());
        }
        let mut r = valid();
        r.steps[0].arguments = json!([]);
        assert!(r.validate().is_err());
        let mut r = valid();
        r.steps[0].check.pointer = String::new();
        assert!(r.validate().is_err());
        let mut r = valid();
        r.steps[0].arguments = json!({"huge": vec![true; 20_000]});
        assert!(r.validate().is_err());
    }
}
