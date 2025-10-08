pub struct SemanticChunker {
    max_chunk_size: usize,
    overlap_size: usize,
}

impl SemanticChunker {
    pub fn new(max_chunk_size: usize, overlap_size: usize) -> Self {
        Self {
            max_chunk_size,
            overlap_size,
        }
    }

    pub fn chunk(&self, text: &str) -> Vec<String> {
        let paragraphs: Vec<&str> = text.split("\n\n").collect();

        let mut chunks = Vec::new();
        let mut current_chunk = String::new();

        for paragraph in paragraphs {
            let paragraph_size = paragraph.len();

            if current_chunk.len() + paragraph_size > self.max_chunk_size
                && !current_chunk.is_empty()
            {
                chunks.push(current_chunk.clone());

                let previous_overlap = self.extract_overlap(&current_chunk);
                current_chunk = previous_overlap;
            }

            if !current_chunk.is_empty() {
                current_chunk.push_str("\n\n");
            }
            current_chunk.push_str(paragraph);
        }

        if !current_chunk.is_empty() {
            chunks.push(current_chunk);
        }

        chunks
    }

    fn extract_overlap(&self, text: &str) -> String {
        if text.len() <= self.overlap_size {
            return text.to_string();
        }

        let start_pos = text.len() - self.overlap_size;

        if let Some(newline_pos) = text[start_pos..].find('\n') {
            text[start_pos + newline_pos + 1..].to_string()
        } else {
            text[start_pos..].to_string()
        }
    }
}

impl Default for SemanticChunker {
    fn default() -> Self {
        Self::new(4000, 200)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_small_text() {
        let chunker = SemanticChunker::new(1000, 100);
        let text = "Small text that fits in one chunk";

        let chunks = chunker.chunk(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn test_paragraph_splitting() {
        let chunker = SemanticChunker::new(100, 20);
        let text = "First paragraph.\n\nSecond paragraph that should be in a different chunk because it's long.\n\nThird paragraph.";

        let chunks = chunker.chunk(text);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn test_overlap() {
        let chunker = SemanticChunker::new(50, 10);
        let text = "First paragraph with some content.\n\nSecond paragraph with more content.\n\nThird paragraph.";

        let chunks = chunker.chunk(text);
        assert!(chunks.len() >= 2);
    }
}
