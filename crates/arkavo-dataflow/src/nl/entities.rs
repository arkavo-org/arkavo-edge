use anyhow::Result;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub name: String,
    pub entity_type: EntityType,
    pub position: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EntityType {
    TaskWindow,
    Model,
    DataSource,
    DataSink,
    Generic,
}

pub struct EntityRecognizer {
    known_entities: HashSet<String>,
    model_names: HashSet<String>,
}

impl EntityRecognizer {
    pub fn new() -> Self {
        let mut recognizer = Self {
            known_entities: HashSet::new(),
            model_names: HashSet::new(),
        };

        // Add known entity patterns
        recognizer.register_common_entities();
        recognizer.register_model_names();

        recognizer
    }

    fn register_common_entities(&mut self) {
        // Common data sources
        self.known_entities.insert("chat".to_string());
        self.known_entities.insert("input".to_string());
        self.known_entities.insert("user_input".to_string());
        self.known_entities.insert("log_stream".to_string());
        self.known_entities.insert("test_results".to_string());
        self.known_entities.insert("error_stream".to_string());
        self.known_entities.insert("api".to_string());
        self.known_entities.insert("webhook".to_string());
        self.known_entities.insert("file_watcher".to_string());

        // Common data sinks
        self.known_entities.insert("output".to_string());
        self.known_entities.insert("console".to_string());
        self.known_entities.insert("debugger".to_string());
        self.known_entities.insert("error_handler".to_string());
        self.known_entities.insert("summary_window".to_string());
        self.known_entities.insert("review_log".to_string());
        self.known_entities.insert("database".to_string());
        self.known_entities.insert("file".to_string());

        // Common processors
        self.known_entities.insert("code_analyzer".to_string());
        self.known_entities.insert("test_runner".to_string());
        self.known_entities.insert("error_analyzer".to_string());
        self.known_entities.insert("transformer".to_string());
        self.known_entities.insert("filter".to_string());
        self.known_entities.insert("router".to_string());
        self.known_entities.insert("aggregator".to_string());
    }

    fn register_model_names(&mut self) {
        self.model_names.insert("gpt-4".to_string());
        self.model_names.insert("gpt-4o".to_string());
        self.model_names.insert("claude".to_string());
        self.model_names.insert("llama".to_string());
        self.model_names.insert("mistral".to_string());
        self.model_names.insert("devstral".to_string());
    }

    /// Extracts entities from the input text.
    ///
    /// # Panics
    ///
    /// Panics if regex capture groups are not properly matched (should not happen with valid regex).
    pub fn extract(&self, input: &str) -> Result<Vec<Entity>> {
        let mut entities = Vec::new();
        let input_lower = input.to_lowercase();
        let words: Vec<&str> = input.split_whitespace().collect();

        // First pass: Look for known entities
        for word in &words {
            let clean_word = self.clean_word(word);
            let clean_lower = clean_word.to_lowercase();

            if self.known_entities.contains(&clean_lower) {
                let position = input_lower.find(&clean_lower).unwrap_or(0);
                entities.push(Entity {
                    name: clean_word,
                    entity_type: self.classify_entity(&clean_lower),
                    position,
                });
            } else if self.model_names.contains(&clean_lower) {
                let position = input_lower.find(&clean_lower).unwrap_or(0);
                entities.push(Entity {
                    name: clean_word,
                    entity_type: EntityType::Model,
                    position,
                });
            }
        }

        // Second pass: Look for compound entities (e.g., "code analyzer")
        for i in 0..words.len().saturating_sub(1) {
            let compound = format!(
                "{} {}",
                self.clean_word(words[i]),
                self.clean_word(words[i + 1])
            )
            .to_lowercase();

            if self.known_entities.contains(&compound.replace(' ', "_")) {
                let position = input_lower.find(&compound).unwrap_or(0);
                entities.push(Entity {
                    name: compound.replace(' ', "_"),
                    entity_type: self.classify_entity(&compound),
                    position,
                });
            }
        }

        // Third pass: Look for quoted entities
        // But skip quoted strings that are likely condition values
        use regex::Regex;
        use std::sync::LazyLock;
        static QUOTED_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"["']([^"']+)["']"#).unwrap());
        let quoted_regex = &*QUOTED_RE;
        let condition_keywords = ["contains", "equals", "matches", "starts with", "ends with"];

        for cap in quoted_regex.captures_iter(input) {
            if let Some(match_) = cap.get(1) {
                let entity_name = match_.as_str().to_string();
                // Get the position of the whole match (including quotes)
                let quote_position = cap.get(0).unwrap().start();

                // Check if this quoted string appears after a condition keyword
                let text_before = &input[..quote_position];
                let is_condition_value = condition_keywords.iter().any(|kw| {
                    text_before.to_lowercase().rfind(kw).is_some_and(|kw_pos| {
                        // Check if there's only whitespace between keyword and quote
                        let between = &text_before[kw_pos + kw.len()..];
                        between.trim().is_empty()
                    })
                });

                // Only add as entity if it's not a condition value and not already found
                if !is_condition_value && !entities.iter().any(|e| e.name == entity_name) {
                    entities.push(Entity {
                        name: entity_name.replace(' ', "_"),
                        entity_type: EntityType::Generic,
                        position: quote_position,
                    });
                }
            }
        }

        // Fourth pass: Look for "all X" patterns
        let words_lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
        for (i, word) in words_lower.iter().enumerate() {
            if word == "all" && i + 1 < words_lower.len() {
                let next_word = &words_lower[i + 1];
                let entity_name = format!("all_{next_word}");
                let position = input_lower.find(&format!("all {next_word}")).unwrap_or(0);

                // Don't duplicate if already found
                if !entities.iter().any(|e| e.name == entity_name) {
                    entities.push(Entity {
                        name: entity_name,
                        entity_type: EntityType::DataSource,
                        position,
                    });
                }
            }
        }

        // Sort by position for consistent ordering
        entities.sort_by_key(|e| e.position);

        // Remove duplicates
        entities.dedup_by(|a, b| a.name == b.name);

        Ok(entities)
    }

    fn clean_word(&self, word: &str) -> String {
        word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
            .to_string()
    }

    fn classify_entity(&self, name: &str) -> EntityType {
        if name.contains("window") || name.contains("view") {
            EntityType::TaskWindow
        } else if name.contains("stream") || name.contains("input") || name.contains("source") {
            EntityType::DataSource
        } else if name.contains("output") || name.contains("sink") || name.contains("handler") {
            EntityType::DataSink
        } else if self.model_names.contains(name) {
            EntityType::Model
        } else {
            EntityType::Generic
        }
    }

    pub fn register_entity(&mut self, name: String) {
        self.known_entities.insert(name.to_lowercase());
    }
}

impl Default for EntityRecognizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_extraction() {
        let recognizer = EntityRecognizer::new();

        let entities = recognizer.extract("Connect chat to code_analyzer").unwrap();
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].name, "chat");
        assert_eq!(entities[1].name, "code_analyzer");
    }

    #[test]
    fn test_quoted_entity_extraction() {
        let recognizer = EntityRecognizer::new();

        let entities = recognizer
            .extract("Send data from 'user input' to output")
            .unwrap();
        assert!(entities.iter().any(|e| e.name == "user_input"));
        assert!(entities.iter().any(|e| e.name == "output"));
    }

    #[test]
    fn test_all_pattern_extraction() {
        let recognizer = EntityRecognizer::new();

        let entities = recognizer
            .extract("Send errors from all tasks to debugger")
            .unwrap();
        assert!(entities.iter().any(|e| e.name == "all_tasks"));
        assert!(entities.iter().any(|e| e.name == "debugger"));
    }

    #[test]
    fn test_model_name_extraction() {
        let recognizer = EntityRecognizer::new();

        let entities = recognizer.extract("Route to gpt-4 for analysis").unwrap();
        assert!(
            entities
                .iter()
                .any(|e| e.name == "gpt-4" && e.entity_type == EntityType::Model)
        );
    }

    #[test]
    fn test_quoted_value_not_entity() {
        let recognizer = EntityRecognizer::new();

        let entities = recognizer
            .extract("Connect input to output when message contains 'test'")
            .unwrap();
        eprintln!(
            "Entities found: {:?}",
            entities.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        assert_eq!(entities.len(), 2);
        assert!(entities.iter().any(|e| e.name == "input"));
        assert!(entities.iter().any(|e| e.name == "output"));
        assert!(!entities.iter().any(|e| e.name == "test"));
    }
}
