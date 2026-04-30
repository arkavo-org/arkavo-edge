//! Validate a SwarmKit manifest from disk.
//!
//! Reads YAML or JSON depending on file extension. Prints either the parsed
//! kit metadata + computed BLAKE3 kit.id, or a validation error.
//!
//! Usage:
//!   cargo run -p arkavo-swarmkit --example validate_kit -- <path>

use std::path::PathBuf;

use arkavo_swarmkit::{kit_id_for, parse_json, parse_yaml};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: validate_kit <path>");
        std::process::exit(2);
    };
    let path = PathBuf::from(&path);
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", path.display());
            std::process::exit(2);
        }
    };

    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));

    let manifest = if is_json {
        parse_json(&raw)
    } else {
        parse_yaml(&raw)
    };

    match manifest {
        Ok(m) => {
            let computed_id = kit_id_for(&m).expect("hash");
            println!("name:        {}", m.kit.name);
            println!("version:     {}", m.kit.version);
            println!("spec:        {}", m.spec_version);
            println!(
                "authors:     {:?}",
                m.kit.authors.iter().map(|a| &a.did).collect::<Vec<_>>()
            );
            println!("created:     {}", m.kit.created);
            if let Some(exp) = &m.kit.expires {
                println!("expires:     {exp}");
            }
            println!("nonce:       {}", m.kit.nonce);
            println!("kit.id:      {} (declared)", m.kit.id);
            println!("kit.id:      {computed_id} (computed)");
            println!(
                "roles:       {} ({})",
                m.roles.len(),
                m.roles
                    .iter()
                    .map(|r| r.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!(
                "coord:       {:?} / {:?}",
                m.coordination.topology, m.coordination.routing.strategy
            );
            println!(
                "budget:      {}s wallclock, {} tokens, ${:.2}",
                m.constraints.global_budget.max_wallclock_seconds,
                m.constraints.global_budget.max_total_tokens,
                m.constraints.global_budget.max_cost_usd
            );
            println!("egress:      {}", m.constraints.network.egress_allowed);
            if let Some(eval) = &m.evaluation {
                println!("critic:      {}", eval.critic_role);
                println!(
                    "dimensions:  {}",
                    eval.rubric
                        .dimensions
                        .iter()
                        .map(|d| format!("{}@{:.2}/{:.2}", d.name, d.weight, d.threshold))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
            println!("VALID");
        }
        Err(e) => {
            eprintln!("INVALID: {e}");
            std::process::exit(1);
        }
    }
}
