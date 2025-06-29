use anyhow::Result;

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Connect,
    Filter,
    Transform,
    Route,
    Split,
    Merge,
}

pub struct IntentClassifier {
    intent_keywords: Vec<(Vec<&'static str>, Intent)>,
}

impl IntentClassifier {
    pub fn new() -> Self {
        let mut classifier = Self {
            intent_keywords: Vec::new(),
        };

        classifier.register_intent_patterns();
        classifier
    }

    fn register_intent_patterns(&mut self) {
        // Connect patterns
        self.intent_keywords.push((
            vec!["connect", "link", "send", "pipe", "forward", "direct"],
            Intent::Connect,
        ));

        // Filter patterns
        self.intent_keywords.push((
            vec!["filter", "when", "if", "only", "where", "select"],
            Intent::Filter,
        ));

        // Transform patterns
        self.intent_keywords.push((
            vec![
                "transform",
                "convert",
                "format",
                "map",
                "process",
                "extract",
                "enrich",
                "aggregate",
            ],
            Intent::Transform,
        ));

        // Route patterns
        self.intent_keywords
            .push((vec!["route", "dispatch", "distribute"], Intent::Route));

        // Split patterns
        self.intent_keywords.push((
            vec!["split", "broadcast", "replicate", "fork"],
            Intent::Split,
        ));

        // Merge patterns
        self.intent_keywords.push((
            vec!["merge", "combine", "join", "aggregate", "collect"],
            Intent::Merge,
        ));
    }

    pub fn classify(&self, input: &str) -> Result<Intent> {
        let input_lower = input.to_lowercase();
        let words: Vec<&str> = input_lower.split_whitespace().collect();

        // Score each intent based on keyword matches
        let mut intent_scores: Vec<(Intent, usize)> = Vec::new();

        for (keywords, intent) in &self.intent_keywords {
            let mut score = 0;

            for keyword in keywords {
                if words.contains(keyword) {
                    score += 2; // Direct word match
                } else if input_lower.contains(keyword) {
                    score += 1; // Substring match
                }
            }

            if score > 0 {
                intent_scores.push((intent.clone(), score));
            }
        }

        // Additional context clues
        if self.has_filter_context(&input_lower) {
            if let Some(filter_score) = intent_scores
                .iter_mut()
                .find(|(i, _)| matches!(i, Intent::Filter))
            {
                filter_score.1 += 3; // Boost filter intent
            } else {
                intent_scores.push((Intent::Filter, 3));
            }
        }

        if self.has_transform_context(&input_lower) {
            if let Some(transform_score) = intent_scores
                .iter_mut()
                .find(|(i, _)| matches!(i, Intent::Transform))
            {
                transform_score.1 += 2; // Boost transform intent
            }
        }

        // Check for multiple targets (suggests split)
        if self.count_targets(&input_lower) > 1 && input_lower.contains(" to ") {
            if let Some(split_score) = intent_scores
                .iter_mut()
                .find(|(i, _)| matches!(i, Intent::Split))
            {
                split_score.1 += 2;
            } else {
                intent_scores.push((Intent::Split, 2));
            }
        }

        // Sort by score (highest first)
        intent_scores.sort_by_key(|(_, score)| std::cmp::Reverse(*score));

        // Return the highest scoring intent, or default to Connect
        Ok(intent_scores
            .into_iter()
            .next()
            .map_or(Intent::Connect, |(intent, _)| intent))
    }

    fn has_filter_context(&self, input: &str) -> bool {
        // Look for filtering patterns
        let filter_patterns = [
            "contains",
            "equals",
            "matches",
            "starts with",
            "ends with",
            "greater than",
            "less than",
            "error",
            "warning",
            "info",
            "debug",
        ];

        filter_patterns
            .iter()
            .any(|pattern| input.contains(pattern))
    }

    fn has_transform_context(&self, input: &str) -> bool {
        // Look for transformation patterns
        let transform_patterns = [
            "to json",
            "to markdown",
            "to csv",
            "uppercase",
            "lowercase",
            "extract",
            "field",
            "property",
            "summarize",
            "summary",
        ];

        transform_patterns
            .iter()
            .any(|pattern| input.contains(pattern))
    }

    fn count_targets(&self, input: &str) -> usize {
        // Count potential target indicators
        let target_indicators = ["and", ",", "&", "plus"];

        let mut count = 1; // At least one target
        for indicator in &target_indicators {
            count += input.matches(indicator).count();
        }

        count
    }
}

impl Default for IntentClassifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connect_intent() {
        let classifier = IntentClassifier::new();

        assert_eq!(
            classifier.classify("Connect A to B").unwrap(),
            Intent::Connect
        );
        assert_eq!(
            classifier
                .classify("Send messages from input to output")
                .unwrap(),
            Intent::Connect
        );
        assert_eq!(
            classifier.classify("Pipe data to processor").unwrap(),
            Intent::Connect
        );
    }

    #[test]
    fn test_filter_intent() {
        let classifier = IntentClassifier::new();

        assert_eq!(
            classifier.classify("Send only errors to debugger").unwrap(),
            Intent::Filter
        );
        assert_eq!(
            classifier
                .classify("Filter messages that contain 'test'")
                .unwrap(),
            Intent::Filter
        );
        assert_eq!(
            classifier.classify("When level equals error").unwrap(),
            Intent::Filter
        );
    }

    #[test]
    fn test_transform_intent() {
        let classifier = IntentClassifier::new();

        assert_eq!(
            classifier
                .classify("Transform data to JSON format")
                .unwrap(),
            Intent::Transform
        );
        assert_eq!(
            classifier.classify("Convert messages to markdown").unwrap(),
            Intent::Transform
        );
        assert_eq!(
            classifier.classify("Extract fields from payload").unwrap(),
            Intent::Transform
        );
    }

    #[test]
    fn test_split_intent() {
        let classifier = IntentClassifier::new();

        assert_eq!(
            classifier.classify("Split input to A and B").unwrap(),
            Intent::Split
        );
        assert_eq!(
            classifier
                .classify("Broadcast messages to multiple outputs")
                .unwrap(),
            Intent::Split
        );
    }

    #[test]
    fn test_context_boost() {
        let classifier = IntentClassifier::new();

        // "Send" normally suggests Connect, but with filter context should be Filter
        assert_eq!(
            classifier
                .classify("Send messages that contain errors to debugger")
                .unwrap(),
            Intent::Filter
        );
    }
}
