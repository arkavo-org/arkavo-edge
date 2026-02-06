//! Context manifest message types for RLM mode gossip

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Announcement of a context manifest for RLM mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextManifestAnnouncement {
    /// Unique manifest identifier
    pub manifest_id: String,
    /// Agent ID of the manifest holder
    pub holder: String,
    /// Total number of chunks in the manifest
    pub chunk_count: usize,
    /// Total token count across all chunks
    pub total_tokens: usize,
    /// Brief summary of the context content
    pub summary: String,
    /// When the manifest was created
    pub timestamp: DateTime<Utc>,
    /// TTL in seconds (how long this manifest will be available)
    pub ttl_seconds: u64,
    /// Ed25519 signature over the announcement
    pub signature: Vec<u8>,
}

impl ContextManifestAnnouncement {
    /// Create a new unsigned manifest announcement
    #[must_use]
    pub fn new(
        manifest_id: String,
        holder: String,
        chunk_count: usize,
        total_tokens: usize,
        summary: String,
        ttl_seconds: u64,
    ) -> Self {
        Self {
            manifest_id,
            holder,
            chunk_count,
            total_tokens,
            summary,
            timestamp: Utc::now(),
            ttl_seconds,
            signature: Vec::new(),
        }
    }

    /// Get the content bytes for signing
    #[must_use]
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.manifest_id.as_bytes());
        content.extend(self.holder.as_bytes());
        content.extend(self.chunk_count.to_le_bytes());
        content.extend(self.total_tokens.to_le_bytes());
        content.extend(self.summary.as_bytes());
        content.extend(self.timestamp.timestamp().to_le_bytes());
        content.extend(self.ttl_seconds.to_le_bytes());
        content
    }
}

/// Request for specific chunks from a context manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextChunkRequest {
    /// Manifest ID being requested from
    pub manifest_id: String,
    /// Agent ID of the requester
    pub requester: String,
    /// Chunk indices to retrieve
    pub indices: Vec<usize>,
    /// Optional: keyword search instead of specific indices
    pub keywords: Option<Vec<String>>,
}

impl ContextChunkRequest {
    /// Create a new chunk request by indices
    #[cfg(test)]
    #[must_use]
    pub(crate) fn by_indices(manifest_id: String, requester: String, indices: Vec<usize>) -> Self {
        Self {
            manifest_id,
            requester,
            indices,
            keywords: None,
        }
    }

    /// Create a new chunk request by keyword search
    #[cfg(test)]
    #[must_use]
    pub(crate) fn by_keywords(
        manifest_id: String,
        requester: String,
        keywords: Vec<String>,
    ) -> Self {
        Self {
            manifest_id,
            requester,
            indices: Vec::new(),
            keywords: Some(keywords),
        }
    }
}

/// Delivery of requested chunks from a context manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextChunkDelivery {
    /// Manifest ID the chunks belong to
    pub manifest_id: String,
    /// Agent ID of the manifest holder
    pub holder: String,
    /// Delivered chunks with their indices and content
    pub chunks: Vec<ContextChunk>,
}

/// A single chunk from a context manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextChunk {
    /// Chunk index within the manifest
    pub index: usize,
    /// Estimated token count for this chunk
    pub tokens: usize,
    /// Full chunk content
    pub content: String,
    /// Content hints/keywords for searching
    pub hints: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_manifest_announcement() {
        let ann = ContextManifestAnnouncement::new(
            "manifest-123".to_string(),
            "agent-1".to_string(),
            10,
            5000,
            "Test context summary".to_string(),
            3600,
        );
        assert!(ann.signature.is_empty());
        assert!(!ann.content_to_sign().is_empty());
        assert_eq!(ann.chunk_count, 10);
        assert_eq!(ann.total_tokens, 5000);
    }

    #[test]
    fn test_content_to_sign_deterministic() {
        let ann = ContextManifestAnnouncement::new(
            "m1".to_string(),
            "a1".to_string(),
            5,
            100,
            "s".to_string(),
            60,
        );
        let sig1 = ann.content_to_sign();
        let sig2 = ann.content_to_sign();
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_context_chunk_request_by_indices() {
        let req = ContextChunkRequest::by_indices(
            "manifest-123".to_string(),
            "agent-2".to_string(),
            vec![0, 2, 5],
        );
        assert_eq!(req.indices.len(), 3);
        assert!(req.keywords.is_none());
    }

    #[test]
    fn test_context_chunk_request_by_keywords() {
        let search = ContextChunkRequest::by_keywords(
            "manifest-123".to_string(),
            "agent-2".to_string(),
            vec!["auth".to_string(), "login".to_string()],
        );
        assert!(search.indices.is_empty());
        assert_eq!(search.keywords.as_ref().unwrap().len(), 2);
    }
}
