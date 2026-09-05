#![allow(clippy::disallowed_methods)] // tokio::test needs block_on internally

//! Regression coverage for search over placeholder (zero-norm) embeddings.
//!
//! Callers that have no embedding model available -- the `embeddings` feature is
//! off by default -- store and query with an all-zero vector. Cosine distance is
//! undefined there and the backing implementation reports 0.0, so every indexed
//! point ties at the best possible distance and the HNSW greedy walk has no
//! gradient to follow. Its layer assignment is seeded from OS entropy, so before
//! the fix each fresh process built a different graph and `search` returned a
//! different, arbitrarily truncated subset -- which is what made every
//! search-backed `arkavo-session` conversation test flaky.

use arkavo_memory::{HnswConfig, Memory, MemoryStorage};
use chrono::Utc;
use uuid::Uuid;

const MEMORY_COUNT: usize = 16;
/// Each iteration builds a fresh index, and therefore a fresh layer-generator
/// seed. The old code dropped at least one point on roughly a fifth of graphs at
/// this size, so twenty independent graphs fail it with near-certainty.
const GRAPH_SAMPLES: usize = 20;

fn placeholder_memory(idx: usize) -> Memory {
    let now = Utc::now();
    Memory {
        id: Uuid::new_v4(),
        content: format!("message {idx}"),
        metadata: None,
        category: Some("conversation".to_string()),
        embedding: vec![0.0; 384],
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn zero_norm_query_returns_every_match_on_every_fresh_index() {
    for sample in 0..GRAPH_SAMPLES {
        let temp = tempfile::tempdir().expect("temp dir");
        let storage =
            MemoryStorage::with_path(temp.path().join("memories.db"), HnswConfig::default())
                .await
                .expect("storage");

        let mut expected = Vec::with_capacity(MEMORY_COUNT);
        for idx in 0..MEMORY_COUNT {
            let memory = placeholder_memory(idx);
            expected.push(memory.id);
            storage.store(memory).await.expect("store");
        }

        let results = storage
            .search(
                "session_id:whatever",
                MEMORY_COUNT + 10,
                Some("conversation"),
            )
            .await
            .expect("search");

        let mut returned: Vec<Uuid> = results.into_iter().map(|r| r.memory.id).collect();
        returned.sort();
        expected.sort();

        assert_eq!(
            returned, expected,
            "sample {sample}: search over placeholder embeddings must return every \
             stored memory, not an index-dependent subset"
        );
    }
}

#[tokio::test]
async fn zero_norm_query_orders_newest_first() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = MemoryStorage::with_path(temp.path().join("memories.db"), HnswConfig::default())
        .await
        .expect("storage");

    // Distinct, deliberately out-of-insertion-order timestamps: recency, not
    // insertion order, is the documented ordering for this path.
    let base = Utc::now();
    let offsets = [30i64, 10, 50, 20];
    let mut ids = Vec::new();
    for (idx, offset) in offsets.iter().enumerate() {
        let mut memory = placeholder_memory(idx);
        memory.created_at = base + chrono::Duration::seconds(*offset);
        memory.updated_at = memory.created_at;
        ids.push((memory.id, *offset));
        storage.store(memory).await.expect("store");
    }

    let results = storage
        .search("anything", 10, Some("conversation"))
        .await
        .expect("search");

    ids.sort_by_key(|(_, offset)| std::cmp::Reverse(*offset));
    let expected: Vec<Uuid> = ids.into_iter().map(|(id, _)| id).collect();
    let returned: Vec<Uuid> = results.into_iter().map(|r| r.memory.id).collect();

    assert_eq!(returned, expected);
}

#[tokio::test]
async fn zero_norm_query_respects_category_filter() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = MemoryStorage::with_path(temp.path().join("memories.db"), HnswConfig::default())
        .await
        .expect("storage");

    let wanted = placeholder_memory(0);
    let wanted_id = wanted.id;
    storage.store(wanted).await.expect("store");

    let mut other = placeholder_memory(1);
    other.category = Some("notes".to_string());
    storage.store(other).await.expect("store");

    let results = storage
        .search("anything", 10, Some("conversation"))
        .await
        .expect("search");

    let returned: Vec<Uuid> = results.into_iter().map(|r| r.memory.id).collect();
    assert_eq!(returned, vec![wanted_id]);
}
