#![allow(clippy::disallowed_methods)] // tokio::test needs block_on internally

//! Spec traceability tests for memory storage and the context ledger.
//!
//! The `embeddings` feature is off in this configuration, so the embedding
//! service hands back placeholder zero vectors and `search` ranks by recency.
//! Tests that need a real vector therefore supply the embedding directly on
//! the stored `Memory`, which is the same path `store_with_replay_state` uses
//! for model-generated embeddings.

use arkavo_memory::{ContextLedger, HnswConfig, Memory, MemoryStorage};
use arkavo_test_macros::spec;
use chrono::{Duration, Utc};
use std::sync::Arc;
use uuid::Uuid;

fn memory_with_embedding(content: &str, category: Option<&str>, embedding: Vec<f32>) -> Memory {
    let now = Utc::now();
    Memory {
        id: Uuid::new_v4(),
        content: content.to_string(),
        metadata: None,
        category: category.map(str::to_string),
        embedding,
        created_at: now,
        updated_at: now,
    }
}

fn placeholder_memory(content: &str, category: &str) -> Memory {
    memory_with_embedding(content, Some(category), vec![0.0; 384])
}

fn pointer_id(pointer: &str) -> String {
    pointer
        .rsplit_once("ID: ")
        .map(|(_, rest)| rest.trim_end_matches(']').to_string())
        .expect("offload pointer must embed the fragment UUID")
}

#[spec("MEM-001")]
#[tokio::test]
async fn store_persists_content_embedding_and_domain_across_reopen() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("memories.db");
    let storage = MemoryStorage::with_path(db_path.clone(), HnswConfig::default())
        .await
        .expect("storage");

    let embedding: Vec<f32> = (0..384).map(|i| (i as f32) / 384.0).collect();
    let memory = memory_with_embedding("domain fact", Some("domain-alpha"), embedding.clone());
    let id = memory.id;
    storage.store(memory).await.expect("store");

    let stored = storage.get(id).await.expect("get");
    assert_eq!(stored.content, "domain fact");
    assert_eq!(stored.category.as_deref(), Some("domain-alpha"));
    assert_eq!(
        stored.embedding, embedding,
        "embedding must round-trip through storage exactly"
    );

    drop(storage);

    let reopened = MemoryStorage::with_path(db_path, HnswConfig::default())
        .await
        .expect("reopen");
    let persisted = reopened.get(id).await.expect("get after reopen");
    assert_eq!(persisted.content, "domain fact");
    assert_eq!(persisted.category.as_deref(), Some("domain-alpha"));
    assert_eq!(persisted.embedding, embedding);
}

#[spec("MEM-001")]
#[tokio::test]
async fn stored_memory_is_searchable_and_scoped_to_its_domain() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = MemoryStorage::with_path(temp.path().join("memories.db"), HnswConfig::default())
        .await
        .expect("storage");

    let alpha = placeholder_memory("alpha fact", "domain-alpha");
    let alpha_id = alpha.id;
    storage.store(alpha).await.expect("store alpha");

    let beta = placeholder_memory("beta fact", "domain-beta");
    let beta_id = beta.id;
    storage.store(beta).await.expect("store beta");

    let found = storage
        .search("alpha fact", 10, Some("domain-alpha"))
        .await
        .expect("search");
    let found_ids: Vec<Uuid> = found.iter().map(|r| r.memory.id).collect();
    assert!(
        found_ids.contains(&alpha_id),
        "stored memory must be retrievable through search"
    );
    assert!(
        !found_ids.contains(&beta_id),
        "search must stay scoped to the requested domain"
    );

    let listed = storage
        .list_by_category("domain-alpha", 10)
        .await
        .expect("list by domain");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, alpha_id);
}

#[spec("MEM-002")]
#[tokio::test]
async fn search_returns_top_k_most_recent_results() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = MemoryStorage::with_path(temp.path().join("memories.db"), HnswConfig::default())
        .await
        .expect("storage");

    let base = Utc::now();
    let mut ids = Vec::new();
    for idx in 0..5 {
        let mut memory = placeholder_memory(&format!("fact {idx}"), "domain-alpha");
        memory.created_at = base + Duration::seconds(idx);
        memory.updated_at = memory.created_at;
        ids.push(memory.id);
        storage.store(memory).await.expect("store");
    }

    let results = storage
        .search("fact", 3, Some("domain-alpha"))
        .await
        .expect("search");

    assert_eq!(results.len(), 3, "search must honor the top-K limit");
    let returned: Vec<Uuid> = results.iter().map(|r| r.memory.id).collect();
    let expected: Vec<Uuid> = ids.iter().rev().take(3).copied().collect();
    assert_eq!(returned, expected, "top-K must be the most recent memories");
}

#[spec("MEM-003")]
#[spec("CTX-010")]
#[tokio::test]
async fn offload_appends_turn_and_preserves_prior_context() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = Arc::new(
        MemoryStorage::with_path(temp.path().join("memories.db"), HnswConfig::default())
            .await
            .expect("storage"),
    );
    let ledger = ContextLedger::new(Arc::clone(&storage));

    let pointer1 = ledger
        .offload("first turn", "turn one", "test")
        .await
        .expect("offload first");
    let pointer2 = ledger
        .offload("second turn", "turn two", "test")
        .await
        .expect("offload second");

    assert!(pointer1.contains("turn one"));
    let id1 = pointer_id(&pointer1);
    let id2 = pointer_id(&pointer2);

    assert_eq!(
        ledger.restore(&id1).await.expect("restore first"),
        "first turn",
        "prior context must survive later appends"
    );
    assert_eq!(
        ledger.restore(&id2).await.expect("restore second"),
        "second turn"
    );

    let fragment = storage
        .get(Uuid::parse_str(&id1).expect("uuid"))
        .await
        .expect("fragment");
    assert_eq!(
        fragment.category.as_deref(),
        Some(MemoryStorage::CATEGORY_LEDGER_FRAGMENT)
    );
    let age = Utc::now() - fragment.created_at;
    assert!(
        age >= Duration::zero() && age < Duration::minutes(1),
        "append must record a current timestamp, got age {age}"
    );
}

#[spec("MEM-003")]
#[spec("CTX-010")]
#[tokio::test]
async fn offloaded_turn_persists_across_restart() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("memories.db");

    let id = {
        let storage = Arc::new(
            MemoryStorage::with_path(db_path.clone(), HnswConfig::default())
                .await
                .expect("storage"),
        );
        let ledger = ContextLedger::new(storage);
        let pointer = ledger
            .offload("durable turn", "durable", "test")
            .await
            .expect("offload");
        pointer_id(&pointer)
    };

    let storage = Arc::new(
        MemoryStorage::with_path(db_path, HnswConfig::default())
            .await
            .expect("reopen"),
    );
    let ledger = ContextLedger::new(storage);
    assert_eq!(
        ledger.restore(&id).await.expect("restore after restart"),
        "durable turn"
    );
}

#[spec("MEM-004")]
#[tokio::test]
async fn recent_context_respects_limit_and_recency_order() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = MemoryStorage::with_path(temp.path().join("memories.db"), HnswConfig::default())
        .await
        .expect("storage");

    let mut ids = Vec::new();
    for idx in 0..4 {
        let id = Uuid::new_v4();
        storage
            .store_ledger_fragment(
                id,
                format!("turn {idx}"),
                format!("summary {idx}"),
                "test".to_string(),
            )
            .await
            .expect("store fragment");
        ids.push(id);
    }

    let recent = storage
        .list_by_category(MemoryStorage::CATEGORY_LEDGER_FRAGMENT, 2)
        .await
        .expect("list recent");

    assert_eq!(recent.len(), 2, "context retrieval must honor the limit");
    let returned: Vec<Uuid> = recent.iter().map(|m| m.id).collect();
    let expected: Vec<Uuid> = ids.iter().rev().take(2).copied().collect();
    assert_eq!(
        returned, expected,
        "recent context must come back newest-first"
    );
}

#[spec("MEM-004")]
#[tokio::test]
async fn empty_ledger_returns_empty_context() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = MemoryStorage::with_path(temp.path().join("memories.db"), HnswConfig::default())
        .await
        .expect("storage");

    let recent = storage
        .list_by_category(MemoryStorage::CATEGORY_LEDGER_FRAGMENT, 10)
        .await
        .expect("list on empty ledger");

    assert!(recent.is_empty());
}
