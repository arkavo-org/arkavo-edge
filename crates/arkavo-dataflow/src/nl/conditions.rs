use crate::dsl::{Condition, ConditionOperator};
use crate::nl::Intent;
use anyhow::Result;
use serde_json::Value;

pub struct ConditionParser {
    operator_patterns: Vec<(Vec<&'static str>, ConditionOperator)>,
}

impl ConditionParser {
    pub fn new() -> Self {
        let mut parser = Self {
            operator_patterns: Vec::new(),
        };

        parser.register_operator_patterns();
        parser
    }

    fn register_operator_patterns(&mut self) {
        self.operator_patterns.push((
            vec!["contains", "containing", "includes", "including"],
            ConditionOperator::Contains,
        ));

        self.operator_patterns.push((
            vec!["equals", "equal to", "is", "==", "eq"],
            ConditionOperator::Equals,
        ));

        self.operator_patterns.push((
            vec!["matches", "matching", "match"],
            ConditionOperator::Matches,
        ));

        self.operator_patterns.push((
            vec!["greater than", "more than", ">", "gt", "above"],
            ConditionOperator::Gt,
        ));

        self.operator_patterns.push((
            vec!["less than", "below", "<", "lt", "under"],
            ConditionOperator::Lt,
        ));

        self.operator_patterns.push((
            vec!["starts with", "beginning with", "starts"],
            ConditionOperator::StartsWith,
        ));

        self.operator_patterns.push((
            vec!["ends with", "ending with", "ends"],
            ConditionOperator::EndsWith,
        ));

        self.operator_patterns.push((
            vec!["exists", "present", "has", "defined"],
            ConditionOperator::Exists,
        ));

        self.operator_patterns.push((
            vec!["not exists", "missing", "absent", "undefined"],
            ConditionOperator::NotExists,
        ));
    }

    pub fn parse(&self, input: &str, intent: &Intent) -> Result<Vec<Condition>> {
        let mut conditions = Vec::new();
        let input_lower = input.to_lowercase();

        // Extract conditions based on intent
        match intent {
            Intent::Filter | Intent::Connect => {
                // Look for explicit filter conditions
                let explicit = self.extract_filter_conditions(&input_lower)?;
                conditions.extend(explicit);

                // Look for implicit conditions (e.g., "errors" implies level=error)
                let implicit = self.extract_implicit_conditions(&input_lower)?;
                conditions.extend(implicit);
            }
            _ => {
                // Other intents might have conditions in their rules
                conditions.extend(self.extract_filter_conditions(&input_lower)?);
            }
        }

        Ok(conditions)
    }

    fn extract_filter_conditions(&self, input: &str) -> Result<Vec<Condition>> {
        let mut conditions = Vec::new();

        // Look for "when" or "if" clauses
        let condition_start = input
            .find("when ")
            .or_else(|| input.find("if "))
            .or_else(|| input.find("where "))
            .or_else(|| input.find("that "));

        if let Some(start) = condition_start {
            let condition_text = &input[start..];

            // Try to parse the condition
            if let Some(condition) = self.parse_single_condition(condition_text)? {
                conditions.push(condition);
            }

            // If we found a condition in a when/if clause, don't look for more operator patterns
            // to avoid duplicates
            if !conditions.is_empty() {
                return Ok(conditions);
            }
        }

        // Only look for operator patterns if we didn't find a when/if clause
        for (patterns, operator) in &self.operator_patterns {
            for pattern in patterns {
                // For short patterns like "gt", "lt", ensure word boundaries
                let needs_word_boundary = pattern.len() <= 2;

                if let Some(pos) = input.find(pattern) {
                    // Check word boundaries for short operators
                    if needs_word_boundary {
                        let before_ok = pos == 0
                            || !input.chars().nth(pos - 1).unwrap_or(' ').is_alphanumeric();
                        let after_pos = pos + pattern.len();
                        let after_ok = after_pos >= input.len()
                            || !input
                                .chars()
                                .nth(after_pos)
                                .unwrap_or(' ')
                                .is_alphanumeric();

                        if !before_ok || !after_ok {
                            continue; // Skip if not at word boundary
                        }
                    }

                    // Extract field and value around the operator
                    if let Some(condition) =
                        self.extract_condition_parts(input, pos, pattern, operator.clone())?
                    {
                        // Avoid duplicates
                        if !conditions
                            .iter()
                            .any(|c| c.field == condition.field && c.operator == condition.operator)
                        {
                            conditions.push(condition);
                        }
                    }
                }
            }
        }

        Ok(conditions)
    }

    fn extract_implicit_conditions(&self, input: &str) -> Result<Vec<Condition>> {
        let mut conditions = Vec::new();
        let words: Vec<&str> = input.split_whitespace().collect();

        // Common implicit patterns - check for whole words to avoid false matches
        for word in &words {
            let word_lower = word.to_lowercase();

            if word_lower == "error" || word_lower == "errors" {
                conditions.push(Condition {
                    field: "level".to_string(),
                    operator: ConditionOperator::Equals,
                    value: Value::String("error".to_string()),
                });
                break; // Only add once
            } else if word_lower == "warning" || word_lower == "warnings" {
                conditions.push(Condition {
                    field: "level".to_string(),
                    operator: ConditionOperator::Equals,
                    value: Value::String("warning".to_string()),
                });
                break; // Only add once
            }
        }

        if input.contains("failed") || input.contains("failure") {
            conditions.push(Condition {
                field: "status".to_string(),
                operator: ConditionOperator::Equals,
                value: Value::String("failed".to_string()),
            });
        }

        if input.contains("success") || input.contains("successful") {
            conditions.push(Condition {
                field: "status".to_string(),
                operator: ConditionOperator::Equals,
                value: Value::String("success".to_string()),
            });
        }

        Ok(conditions)
    }

    fn parse_single_condition(&self, text: &str) -> Result<Option<Condition>> {
        // Try to parse conditions like "when message contains 'error'"
        let words: Vec<&str> = text.split_whitespace().collect();

        if words.len() < 3 {
            return Ok(None);
        }

        // Find the operator
        for (patterns, operator) in &self.operator_patterns {
            for pattern in patterns {
                if let Some(op_pos) = words.iter().position(|&w| w == *pattern) {
                    // Get field (word before operator)
                    let field = if op_pos > 1 {
                        words[op_pos - 1].to_string()
                    } else if op_pos == 1 && !words.is_empty() {
                        words[0].to_string()
                    } else {
                        "message".to_string() // Default field
                    };

                    // Get value (words after operator)
                    let value = if op_pos + 1 < words.len() {
                        let value_text = words[op_pos + 1..].join(" ");
                        self.parse_value(&value_text)
                    } else {
                        Value::Bool(true) // For exists operator
                    };

                    return Ok(Some(Condition {
                        field: field
                            .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
                            .to_string(),
                        operator: operator.clone(),
                        value,
                    }));
                }
            }
        }

        Ok(None)
    }

    fn extract_condition_parts(
        &self,
        input: &str,
        operator_pos: usize,
        operator_text: &str,
        operator: ConditionOperator,
    ) -> Result<Option<Condition>> {
        // Get text before and after operator
        let before = &input[..operator_pos];
        let after = &input[operator_pos + operator_text.len()..];

        // Extract field name (last word before operator)
        let field = before
            .split_whitespace()
            .last()
            .unwrap_or("message")
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
            .to_string();

        // Extract value (handle quoted strings and single words)
        let value = if let Some(quoted) = self.extract_quoted_value(after) {
            Value::String(quoted)
        } else if let Some(word) = after.split_whitespace().next() {
            self.parse_value(word)
        } else {
            return Ok(None);
        };

        Ok(Some(Condition {
            field,
            operator,
            value,
        }))
    }

    fn extract_quoted_value(&self, text: &str) -> Option<String> {
        // Look for single or double quoted strings
        use regex::Regex;
        use std::sync::LazyLock;

        static SINGLE_QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"'([^']+)'").unwrap());
        static DOUBLE_QUOTED: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#""([^"]+)""#).unwrap());

        if let Some(caps) = SINGLE_QUOTED.captures(text) {
            return Some(caps[1].to_string());
        }

        if let Some(caps) = DOUBLE_QUOTED.captures(text) {
            return Some(caps[1].to_string());
        }

        None
    }

    fn parse_value(&self, text: &str) -> Value {
        let cleaned = text.trim().trim_matches(|c: char| c == '\'' || c == '"');

        // Try to parse as number
        if let Ok(num) = cleaned.parse::<f64>() {
            Value::Number(
                serde_json::Number::from_f64(num).unwrap_or_else(|| serde_json::Number::from(0)),
            )
        } else if let Ok(bool_val) = cleaned.parse::<bool>() {
            Value::Bool(bool_val)
        } else {
            Value::String(cleaned.to_string())
        }
    }
}

impl Default for ConditionParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_explicit_condition() {
        let parser = ConditionParser::new();
        let conditions = parser
            .parse(
                "Send to output when message contains 'error'",
                &Intent::Filter,
            )
            .unwrap();

        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].field, "message");
        assert_eq!(conditions[0].operator, ConditionOperator::Contains);
        assert_eq!(conditions[0].value, Value::String("error".to_string()));
    }

    #[test]
    fn test_parse_implicit_condition() {
        let parser = ConditionParser::new();
        let conditions = parser
            .parse("Send errors to debugger", &Intent::Filter)
            .unwrap();

        assert!(!conditions.is_empty());
        assert!(conditions.iter().any(|c| c.field == "level"
            && c.operator == ConditionOperator::Equals
            && c.value == Value::String("error".to_string())));
    }

    #[test]
    fn test_parse_numeric_condition() {
        let parser = ConditionParser::new();
        let conditions = parser
            .parse("Filter when count greater than 100", &Intent::Filter)
            .unwrap();

        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].field, "count");
        assert_eq!(conditions[0].operator, ConditionOperator::Gt);
        assert!(matches!(conditions[0].value, Value::Number(_)));
    }

    #[test]
    fn test_multiple_conditions() {
        let parser = ConditionParser::new();
        let conditions = parser
            .parse(
                "Send messages when level equals 'error' and status equals 'failed'",
                &Intent::Filter,
            )
            .unwrap();

        assert!(conditions.len() >= 2);
    }
}
