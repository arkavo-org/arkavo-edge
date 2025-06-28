use crate::dsl::{Blueprint, Condition, Link, Node, Rule, Transform, TransformType};
use crate::nl::{ConditionParser, EntityRecognizer, Intent, IntentClassifier};
use anyhow::Result;
use semver::Version;
use std::collections::HashMap;

pub struct NLParser {
    entity_recognizer: EntityRecognizer,
    intent_classifier: IntentClassifier,
    condition_parser: ConditionParser,
}

#[derive(Debug)]
pub struct ParsedIntent {
    pub intent: Intent,
    pub source_entities: Vec<String>,
    pub target_entities: Vec<String>,
    pub conditions: Vec<Condition>,
    pub transform_type: Option<TransformType>,
    pub parameters: HashMap<String, serde_json::Value>,
}

impl NLParser {
    pub fn new() -> Self {
        Self {
            entity_recognizer: EntityRecognizer::new(),
            intent_classifier: IntentClassifier::new(),
            condition_parser: ConditionParser::new(),
        }
    }

    pub fn parse_to_blueprint(&self, input: &str) -> Result<Blueprint> {
        let parsed = self.parse(input)?;
        self.create_blueprint_from_intent(parsed)
    }

    pub fn parse(&self, input: &str) -> Result<ParsedIntent> {
        // 1. Extract entities (sources and targets)
        let entities = self.entity_recognizer.extract(input)?;

        // 2. Classify intent
        let intent = self.intent_classifier.classify(input)?;

        // 3. Parse conditions based on intent
        let conditions = self.condition_parser.parse(input, &intent)?;

        // 4. Extract transform type if applicable
        let transform_type = self.extract_transform_type(input, &intent);

        // 5. Extract additional parameters
        let parameters = self.extract_parameters(input);

        // Separate source and target entities
        let (source_entities, target_entities) = self.separate_entities(&entities, input);

        Ok(ParsedIntent {
            intent,
            source_entities,
            target_entities,
            conditions,
            transform_type,
            parameters,
        })
    }

    fn create_blueprint_from_intent(&self, parsed: ParsedIntent) -> Result<Blueprint> {
        let mut blueprint = Blueprint {
            version: Version::new(1, 0, 0),
            name: Some(format!("{:?} Pipeline", parsed.intent)),
            description: None,
            nodes: Vec::new(),
            links: Vec::new(),
            metadata: None,
        };

        // Create nodes based on entities
        for source in &parsed.source_entities {
            blueprint.add_node(Node::source(source));
        }

        for target in &parsed.target_entities {
            blueprint.add_node(Node::sink(target));
        }

        // Create links with rules
        for source in &parsed.source_entities {
            for target in &parsed.target_entities {
                let mut link = Link::new(source, target);

                // Add rule based on intent
                match parsed.intent {
                    Intent::Connect => {
                        if !parsed.conditions.is_empty() {
                            link = link.with_rule(Rule::filter(parsed.conditions.clone()));
                        }
                    }
                    Intent::Filter => {
                        link = link.with_rule(Rule::filter(parsed.conditions.clone()));
                    }
                    Intent::Transform => {
                        if let Some(transform_type) = parsed.transform_type.clone() {
                            let transform = Transform {
                                transform_type,
                                params: parsed.parameters.clone(),
                            };
                            link = link.with_rule(Rule::transform(transform));
                        }
                    }
                    Intent::Route => {
                        // Routing handled by multiple links
                    }
                    Intent::Split => {
                        // Would create a router node
                        let router_id = format!("{}_router", source);
                        blueprint.add_node(Node::router(&router_id));
                        blueprint.add_link(Link::new(source, &router_id));

                        for target in &parsed.target_entities {
                            blueprint.add_link(Link::new(&router_id, target));
                        }
                        continue; // Skip the direct link
                    }
                    Intent::Merge => {
                        // Would create a merger node
                        let merger_id = format!("{}_merger", target);
                        blueprint
                            .add_node(Node::transform(&merger_id).with_param("type", "merger"));

                        for source in &parsed.source_entities {
                            blueprint.add_link(Link::new(source, &merger_id));
                        }
                        blueprint.add_link(Link::new(&merger_id, target));
                        continue; // Skip the direct link
                    }
                }

                blueprint.add_link(link);
            }
        }

        Ok(blueprint)
    }

    fn separate_entities(
        &self,
        entities: &[crate::nl::Entity],
        input: &str,
    ) -> (Vec<String>, Vec<String>) {
        let mut sources = Vec::new();
        let mut targets = Vec::new();

        // Look for directional keywords
        let input_lower = input.to_lowercase();
        let to_position = input_lower.find(" to ");
        let from_position = input_lower.find("from ");

        for entity in entities {
            let entity_position = input_lower.find(&entity.name.to_lowercase()).unwrap_or(0);

            if let Some(to_pos) = to_position {
                if entity_position < to_pos {
                    sources.push(entity.name.clone());
                } else {
                    targets.push(entity.name.clone());
                }
            } else if let Some(from_pos) = from_position {
                if entity_position > from_pos && entity_position < from_pos + 50 {
                    sources.push(entity.name.clone());
                } else {
                    targets.push(entity.name.clone());
                }
            } else {
                // Default: first entity is source, rest are targets
                if sources.is_empty() {
                    sources.push(entity.name.clone());
                } else {
                    targets.push(entity.name.clone());
                }
            }
        }

        (sources, targets)
    }

    fn extract_transform_type(&self, input: &str, intent: &Intent) -> Option<TransformType> {
        if !matches!(intent, Intent::Transform) {
            return None;
        }

        let input_lower = input.to_lowercase();

        if input_lower.contains("extract") {
            Some(TransformType::Extract)
        } else if input_lower.contains("format") || input_lower.contains("markdown") {
            Some(TransformType::Format)
        } else if input_lower.contains("aggregate")
            || input_lower.contains("sum")
            || input_lower.contains("count")
        {
            Some(TransformType::Aggregate)
        } else if input_lower.contains("enrich") || input_lower.contains("add") {
            Some(TransformType::Enrich)
        } else if input_lower.contains("map") {
            Some(TransformType::Map)
        } else if input_lower.contains("reduce") {
            Some(TransformType::Reduce)
        } else {
            Some(TransformType::Map) // Default
        }
    }

    fn extract_parameters(&self, input: &str) -> HashMap<String, serde_json::Value> {
        let mut params = HashMap::new();

        // Extract template type for formatting
        if input.to_lowercase().contains("markdown") {
            params.insert("template".to_string(), serde_json::json!("markdown"));
        } else if input.to_lowercase().contains("json") {
            params.insert("template".to_string(), serde_json::json!("json"));
        }

        // Extract aggregation type
        if input.to_lowercase().contains("sum") {
            params.insert("aggregation".to_string(), serde_json::json!("sum"));
        } else if input.to_lowercase().contains("count") {
            params.insert("aggregation".to_string(), serde_json::json!("count"));
        } else if input.to_lowercase().contains("average") || input.to_lowercase().contains("avg") {
            params.insert("aggregation".to_string(), serde_json::json!("avg"));
        }

        params
    }
}

impl Default for NLParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_connection() {
        let parser = NLParser::new();
        let result = parser.parse("Connect chat to code_analyzer").unwrap();

        assert_eq!(result.source_entities, vec!["chat"]);
        assert_eq!(result.target_entities, vec!["code_analyzer"]);
        assert!(matches!(result.intent, Intent::Connect));
    }

    #[test]
    fn test_parse_filter_connection() {
        let parser = NLParser::new();
        let result = parser
            .parse("Send errors from log_stream to error_handler")
            .unwrap();

        assert!(matches!(result.intent, Intent::Filter));
        assert!(!result.conditions.is_empty());
    }

    #[test]
    fn test_create_blueprint() {
        let parser = NLParser::new();
        let blueprint = parser
            .parse_to_blueprint("Connect input to output when message contains 'test'")
            .unwrap();

        assert_eq!(blueprint.nodes.len(), 2);
        assert_eq!(blueprint.links.len(), 1);
        assert!(blueprint.links[0].rule.is_some());
    }
}
