//! Run-level cost ceiling stops between batches and marks the ledger.

// #[tokio::test] expands to Runtime::block_on; harmless in tests.
#![allow(clippy::disallowed_methods)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use arkavo_shadow_consolidation::{Config, execute};
use arkavo_test_macros::spec;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A valid Fable response whose single call costs $0.01 at Fable list price.
const SUCCESS_BODY: &str = r#"{"content":[{"type":"text","text":"{\"lesson\":{\"condition\":\"food is low\",\"action\":\"set_priority(Growing)\",\"expected_outcome\":\"food recovers\",\"confidence\":0.85},\"tightenings\":[]}"}],"usage":{"input_tokens":1000,"output_tokens":0}}"#;

/// Read headers plus the content-length body so the client never sees a reset
/// mid-request.
async fn read_request(sock: &mut tokio::net::TcpStream) {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = sock.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&data[..pos]).to_lowercase();
            let len = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if data.len() >= pos + 4 + len {
                return;
            }
        }
    }
}

/// Serve one successful Fable response, then stop accepting. Any accidental
/// second connection will hit a closed listener and fail loudly.
async fn serve_once(body: &'static str) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut sock, _)) = listener.accept().await {
            read_request(&mut sock).await;
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    });
    addr
}

async fn seed_store(dir: &Path, categories: &[(&str, usize)]) -> PathBuf {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    let db = dir.join("learning.db");
    let opts = SqliteConnectOptions::new()
        .filename(&db)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE episodes (id TEXT PRIMARY KEY, task_category TEXT NOT NULL, \
         observation_json TEXT NOT NULL, outcome_json TEXT NOT NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (cat, count) in categories {
        for i in 0..*count {
            sqlx::query(
                "INSERT INTO episodes (id, task_category, observation_json, outcome_json) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(format!("{cat}-{i}"))
            .bind(*cat)
            .bind(r#"{"tools_used":["observe","set_priority"]}"#)
            .bind(r#"{"success":true,"quality_metrics":{"correctness":0.8}}"#)
            .execute(&pool)
            .await
            .unwrap();
        }
    }
    pool.close().await;
    db
}

#[spec("SC-001")]
#[tokio::test]
async fn ceiling_exhausted_mid_run_stops_before_next_batch() {
    // One successful call costs $0.01. With a $0.005 ceiling the run must
    // process the first category, then stop before the second batch starts —
    // never mid-call.
    let addr = serve_once(SUCCESS_BODY).await;
    // SAFETY: this integration-test binary runs only this test, on a
    // current-thread runtime — nothing reads the environment concurrently.
    unsafe {
        std::env::set_var("ANTHROPIC_API_KEY", "test-key");
        std::env::set_var("ANTHROPIC_BASE_URL", format!("http://{addr}"));
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let db = seed_store(tmp.path(), &[("colony", 3), ("nav", 3)]).await;
    let out = tmp.path().join("out");
    let cfg = Config {
        db_path: db,
        out_dir: out.clone(),
        model: "claude-fable-5".into(),
        min_episodes: 3,
        max_tokens: 8192,
        effort: "high".into(),
        max_run_cost_usd: 0.005,
        dry_run: false,
    };

    let summary = execute(cfg).await.unwrap();
    assert!(summary.budget_exhausted);
    assert_eq!(summary.categories_consolidated, 1);
    assert_eq!(summary.lessons, 1);
    assert!((summary.total_cost_usd - 0.01).abs() < 1e-12);

    for name in [
        "lessons.json",
        "proposals.json",
        "cost_ledger.json",
        "findings.json",
        "rejects.json",
    ] {
        assert!(out.join(name).exists(), "missing {name}");
    }

    let ledger: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("cost_ledger.json")).unwrap())
            .unwrap();
    assert!(ledger["run_budget"]["exhausted"].as_bool().unwrap());
    assert_eq!(ledger["categories_budget_stopped"].as_u64().unwrap(), 1);
    assert_eq!(ledger["fable_calls"].as_u64().unwrap(), 1);
    assert!((ledger["total_cost_usd"].as_f64().unwrap() - 0.01).abs() < 1e-12);

    let lessons: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("lessons.json")).unwrap()).unwrap();
    assert_eq!(lessons.as_array().unwrap().len(), 1);

    let rejects: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("rejects.json")).unwrap()).unwrap();
    assert!(rejects.as_array().unwrap().is_empty());
}
