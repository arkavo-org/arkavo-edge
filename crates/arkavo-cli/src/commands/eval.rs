//! `arkavo eval run --contract <path> [--answer id=text ...] [--main]`
//!
//! One-shot eval runner used for local verification and as the Part-2 daemon's
//! core. Uses a FakeOperator until the llama.cpp Operator lands (Part 2).

use arkavo_eval::baseline::MemBaselineStore;
use arkavo_eval::contract::EvalContract;
use arkavo_eval::gate::Preconditions;
use arkavo_eval::operator::FakeOperator;
use arkavo_eval::verdict::{Embedder, VerdictError};
use arkavo_eval::{RunOutcome, run_eval};
use async_trait::async_trait;
use std::collections::HashMap;

struct CharEmbedder;

#[async_trait]
impl Embedder for CharEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError> {
        let mut v = vec![0.0f32; 27];
        for c in text.to_lowercase().chars() {
            if c.is_ascii_lowercase() {
                v[(c as u8 - b'a') as usize] += 1.0;
            } else {
                v[26] += 1.0;
            }
        }
        Ok(v)
    }
}

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut contract_path: Option<String> = None;
    let mut answers: HashMap<String, String> = HashMap::new();
    let mut is_main = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--contract" => {
                i += 1;
                contract_path = args.get(i).cloned();
            }
            "--answer" => {
                i += 1;
                if let Some(kv) = args.get(i) {
                    if let Some((k, v)) = kv.split_once('=') {
                        answers.insert(k.to_string(), v.to_string());
                    }
                }
            }
            "--main" => is_main = true,
            other => return Err(format!("unknown eval arg: {other}").into()),
        }
        i += 1;
    }
    let path = contract_path.ok_or("missing --contract <path>")?;
    let contract: EvalContract = serde_json::from_str(&std::fs::read_to_string(&path)?)?;

    let rt = tokio::runtime::Runtime::new()?;
    let outcome: RunOutcome = rt.block_on(async {
        let store = MemBaselineStore::new();
        let pre = Preconditions {
            weights_present: true,
            weights_attested: true,
            provenance_valid: true,
            baseline_present: !is_main,
        };
        let op = FakeOperator {
            answers,
            tok_s: 100.0,
        };
        run_eval(&contract, &pre, &op, &store, &CharEmbedder, is_main).await
    });

    println!("{}", serde_json::to_string_pretty(&outcome.status)?);
    match outcome.status.check_conclusion() {
        Some("failure") | Some("action_required") => std::process::exit(1),
        _ => Ok(()),
    }
}
