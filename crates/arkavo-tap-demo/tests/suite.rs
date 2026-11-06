//! The acceptance suite, end to end, as `verify-suite` runs it.
#![allow(clippy::disallowed_methods)] // Tokio test entrypoints own their runtime.

use arkavo_tap_demo::suite::{self, Coverage};

#[tokio::test(flavor = "multi_thread")]
async fn every_end_to_end_scenario_passes() {
    let lines = suite::run().await.unwrap();
    let failed: Vec<_> = lines
        .iter()
        .filter_map(|l| match &l.coverage {
            Coverage::Ran(Err(why)) => Some(format!("{} {}: {why}", l.id, l.title)),
            _ => None,
        })
        .collect();
    assert!(failed.is_empty(), "{failed:#?}");
    // Every scenario T01 to T30 is accounted for, run here or named elsewhere.
    for n in 1..=30 {
        let id = format!("T{n:02}");
        assert!(lines.iter().any(|l| l.id == id), "{id} is missing");
    }
    assert!(suite::print(&lines));
}
