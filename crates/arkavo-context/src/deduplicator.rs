use std::collections::HashSet;

pub struct Deduplicator {
    similarity_threshold: f64,
}

impl Deduplicator {
    pub fn new(similarity_threshold: f64) -> Self {
        Self {
            similarity_threshold,
        }
    }

    pub fn deduplicate(&self, chunks: Vec<String>) -> Vec<String> {
        let mut unique_chunks = Vec::new();
        let mut seen_hashes = HashSet::new();

        for chunk in chunks {
            let hash = Self::simple_hash(&chunk);

            if !seen_hashes.contains(&hash) && !self.is_similar_to_any(&chunk, &unique_chunks) {
                seen_hashes.insert(hash);
                unique_chunks.push(chunk);
            }
        }

        unique_chunks
    }

    fn simple_hash(text: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        text.to_lowercase().hash(&mut hasher);
        hasher.finish()
    }

    fn is_similar_to_any(&self, chunk: &str, existing: &[String]) -> bool {
        for existing_chunk in existing {
            if self.similarity(chunk, existing_chunk) >= self.similarity_threshold {
                return true;
            }
        }
        false
    }

    fn similarity(&self, a: &str, b: &str) -> f64 {
        let a_words: HashSet<&str> = a.split_whitespace().collect();
        let b_words: HashSet<&str> = b.split_whitespace().collect();

        if a_words.is_empty() && b_words.is_empty() {
            return 1.0;
        }

        let intersection = a_words.intersection(&b_words).count();
        let union = a_words.union(&b_words).count();

        if union == 0 {
            0.0
        } else {
            intersection as f64 / union as f64
        }
    }
}

impl Default for Deduplicator {
    fn default() -> Self {
        Self::new(0.85)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_duplicates() {
        let dedup = Deduplicator::default();
        let chunks = vec![
            "This is a test".to_string(),
            "This is a test".to_string(),
            "Different text".to_string(),
        ];

        let result = dedup.deduplicate(chunks);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_similar_chunks() {
        let dedup = Deduplicator::new(0.8);
        let chunks = vec![
            "The quick brown fox jumps over the lazy dog".to_string(),
            "The quick brown fox jumps over a lazy dog".to_string(),
            "Completely different content here".to_string(),
        ];

        let result = dedup.deduplicate(chunks);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_no_duplicates() {
        let dedup = Deduplicator::default();
        let chunks = vec![
            "First unique text".to_string(),
            "Second unique text".to_string(),
            "Third unique text".to_string(),
        ];

        let result = dedup.deduplicate(chunks);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_empty_chunks() {
        let dedup = Deduplicator::default();
        let result = dedup.deduplicate(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_single_chunk() {
        let dedup = Deduplicator::default();
        let chunks = vec!["only one".to_string()];
        let result = dedup.deduplicate(chunks);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "only one");
    }

    #[test]
    fn test_order_preservation() {
        let dedup = Deduplicator::default();
        let chunks = vec![
            "alpha beta gamma".to_string(),
            "delta epsilon zeta".to_string(),
            "alpha beta gamma".to_string(),
        ];
        let result = dedup.deduplicate(chunks);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "alpha beta gamma");
        assert_eq!(result[1], "delta epsilon zeta");
    }

    #[test]
    fn test_default_impl() {
        let dedup = Deduplicator::default();
        assert!((dedup.similarity_threshold - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn test_all_identical() {
        let dedup = Deduplicator::default();
        let chunks = vec![
            "same content here".to_string(),
            "same content here".to_string(),
            "same content here".to_string(),
            "same content here".to_string(),
        ];
        let result = dedup.deduplicate(chunks);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "same content here");
    }
}
