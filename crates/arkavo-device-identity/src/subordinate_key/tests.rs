use super::*;
use arkavo_test_macros::spec;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = crate::test_storage_dir().unwrap().join("subordinate");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn workload(name: &str) -> Arc<WorkloadKey> {
    Arc::new(crate::workload_key::load_or_create_at(&scratch(name)).unwrap())
}

#[spec("INGRESS-017")]
#[test]
fn a_kept_key_is_the_same_after_a_restart() {
    let path = scratch("gossip_restart");
    let made = load_or_create_at(&path, Purpose::Gossip).unwrap();
    let reloaded = load_or_create_at(&path, Purpose::Gossip).unwrap();
    assert_eq!(reloaded.secret(), made.secret());
    assert_eq!(reloaded.generation(), made.generation());
    assert!(AgentKeypair::from_bytes(made.secret()).is_ok());
}

#[spec("INGRESS-017")]
#[test]
fn a_key_is_made_for_its_purpose_with_the_time_as_its_generation() {
    let before = unix_now();
    let tdf = load_or_create_at(&scratch("tdf"), Purpose::Tdf).unwrap();
    let keypair = P256SigningKeypair::from_bytes(tdf.secret()).unwrap();
    assert_eq!(tdf.did(), keypair.public_key().to_did_key());
    assert_eq!(tdf.purpose(), Purpose::Tdf);
    assert!((before..=unix_now()).contains(&tdf.generation()));
}

#[spec("INGRESS-017")]
#[test]
fn each_agent_keeps_its_keys_in_its_own_directory() {
    let planner = load_or_create("planner-subordinate", Purpose::Gossip).unwrap();
    let coder = load_or_create("coder-subordinate", Purpose::Gossip).unwrap();
    assert_ne!(planner.secret(), coder.secret());
    let file = crate::agent_dir::agent_dir("planner-subordinate")
        .unwrap()
        .join("gossip_key");
    assert!(file.is_file());
}

#[cfg(unix)]
#[spec("INGRESS-017")]
#[test]
fn a_key_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let path = scratch("mode");
    load_or_create_at(&path, Purpose::Iroh).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[spec("INGRESS-017")]
#[test]
fn a_file_that_does_not_hold_this_key_is_refused_and_kept() {
    let other_purpose = scratch("other_purpose");
    load_or_create_at(&other_purpose, Purpose::Iroh).unwrap();
    let err = load_or_create_at(&other_purpose, Purpose::Gossip).unwrap_err();
    assert!(err.to_string().contains("holds the Iroh key"), "{err}");

    let seed = scratch("bare_seed");
    std::fs::write(&seed, [3u8; 32]).unwrap();
    assert!(load_or_create_at(&seed, Purpose::Gossip).is_err());
    assert_eq!(std::fs::read(&seed).unwrap(), [3u8; 32]);
}

#[spec("INGRESS-017")]
#[test]
fn the_voucher_signs_a_statement_peers_accept() {
    let workload = workload("voucher_workload");
    let gossip = load_or_create_at(&scratch("voucher_gossip"), Purpose::Gossip).unwrap();
    let key = AgentKeypair::from_bytes(gossip.secret())
        .unwrap()
        .public_key()
        .to_did_key();
    assert_eq!(gossip.did(), key);
    let voucher = Voucher::new(workload.clone(), &gossip);
    assert_eq!(voucher.key(), key);

    let now = unix_now();
    let read = arkavo_key_statement::verify(&voucher.statement_at(now).unwrap(), now).unwrap();
    assert_eq!(read.issuer, workload.did());
    assert_eq!(read.key, key);
    assert_eq!(read.purpose, Purpose::Gossip);
    assert_eq!(read.generation, gossip.generation());
    // A peer whose clock runs a little behind still accepts it.
    assert_eq!(read.not_before, now - CLOCK_SKEW);
}

#[spec("INGRESS-017")]
#[test]
fn the_voucher_renews_a_statement_before_it_runs_out() {
    let workload = workload("renew_workload");
    let gossip = load_or_create_at(&scratch("renew_gossip"), Purpose::Gossip).unwrap();
    let voucher = Voucher::new(workload, &gossip);

    let start = 1_800_000_000;
    let first = voucher.statement_at(start).unwrap();
    let renew_at = start + LIFETIME - RENEW_WITHIN;
    assert_eq!(voucher.statement_at(renew_at - 1).unwrap(), first);

    let renewed = voucher.statement_at(renew_at).unwrap();
    assert_ne!(renewed, first);
    let read = arkavo_key_statement::verify(&renewed, start + LIFETIME + 60).unwrap();
    assert_eq!(read.not_after, renew_at + LIFETIME);
}
