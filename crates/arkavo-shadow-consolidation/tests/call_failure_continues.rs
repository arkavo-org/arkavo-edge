//! A failed Fable call must not abort the run: later categories still succeed
//! and all artifacts are written.

// #[tokio::test] expands to Runtime::block_on; harmless in tests.
#![allow(clippy::disallowed_methods)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use arkavo_shadow_consolidation::{Config, execute};
use arkavo_test_macros::spec;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A valid Fable response for the category that succeeds.
const SUCCESS_BODY: &str = r#"{"content":[{"type":"text","text":"{\"lesson\":{\"condition\":\"food is low\",\"action\":\"set_priority(Growing)\",\"expected_outcome\":\"food recovers\",\"confidence\":0.85},\"tightenings\":[]}"}],"usage":{"input_tokens":100,"output_tokens":50}}"#;

/// Read headers plus the content-length body so the client never sees a reset
/// mid-request. Returns the full request data so callers can inspect the body.
async fn read_request(sock: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = sock.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            break;
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
                break;
            }
        }
    }
    data
}

/// Serve 400 for requests whose body mentions "Category: colony" and a
/// successful response for all others. This deterministically fails the first
/// category and lets the run continue to the second.
async fn serve_mixed() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            let data = read_request(&mut sock).await;
            let text = String::from_utf8_lossy(&data);
            let is_colony = text.contains("Category: colony");
            let (status, content_type, body) = if is_colony {
                ("400 BAD", "text/plain", "bad request")
            } else {
                ("200 OK", "application/json", SUCCESS_BODY)
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\n\
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

#[spec("SC-003")]
#[tokio::test]
async fn failed_call_records_reject_and_run_continues_to_successful_batch() {
    // Colony fails with a non-retryable 400; nav succeeds. The run must record
    // the operational failure as a call reject, keep going, and still write the
    // ledger and all artifacts for nav.
    let addr = serve_mixed().await;
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
        max_run_cost_usd: 25.0,
        dry_run: false,
    };

    let summary = execute(cfg).await.unwrap();
    assert_eq!(summary.rejects, 1);
    assert_eq!(summary.categories_consolidated, 1);
    assert_eq!(summary.lessons, 1);
    assert!(!summary.budget_exhausted);

    for name in [
        "lessons.json",
        "proposals.json",
        "cost_ledger.json",
        "findings.json",
        "rejects.json",
    ] {
        assert!(out.join(name).exists(), "missing {name}");
    }

    let rejects: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("rejects.json")).unwrap()).unwrap();
    let rejects = rejects.as_array().unwrap();
    assert_eq!(rejects.len(), 1);
    assert_eq!(rejects[0]["kind"], "call");
    assert!(rejects[0]["reason"].as_str().unwrap().contains("400"));
    assert_eq!(rejects[0]["category"], "colony");

    let ledger: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("cost_ledger.json")).unwrap())
            .unwrap();
    assert!((ledger["contract_conformance_rate"].as_f64().unwrap() - 1.0).abs() < 1e-9);
    assert_eq!(ledger["fable_calls"].as_u64().unwrap(), 1);

    let lessons: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("lessons.json")).unwrap()).unwrap();
    let lessons = lessons.as_array().unwrap();
    assert_eq!(lessons.len(), 1);
    assert_eq!(lessons[0]["category"], "nav");
}
