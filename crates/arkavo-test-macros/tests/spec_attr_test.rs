use arkavo_test_macros::spec;

#[spec("HRM-003")]
#[test]
fn test_single_id() {
    assert!(true);
}

#[spec("HRM-003", "HRM-004")]
#[test]
fn test_multiple_ids() {
    assert!(true);
}

#[spec("GOSSIP-005")]
#[tokio::test]
async fn test_async_compat() {
    assert!(true);
}
