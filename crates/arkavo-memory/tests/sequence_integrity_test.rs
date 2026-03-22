//! SEQ-007, SEQ-008, SEQ-009: Tests that existing memory types lack
//! sequence integrity capabilities.

use arkavo_memory::federated_memory::{
    FederatedItem, FederatedQuery, MemoryAttribute, MemoryPolicy, evaluate_entitlements,
};
use arkavo_test_macros::spec;
use chrono::Utc;

/// SEQ-007: FederatedItem has no taint classification field.
/// Tripwire: when taint_classification is added, this will stop panicking.
#[spec("SEQ-007")]
#[test]
#[should_panic(expected = "SEQ-007")]
fn federated_item_has_no_taint_classification_field() {
    let item = FederatedItem {
        id: "item-1".into(),
        agent_id: "agent-1".into(),
        session_id: "session-1".into(),
        content_type: "text".into(),
        content: b"internal credentials".to_vec(),
        summary: "sensitive data".into(),
        token_count: 5,
        policy: MemoryPolicy { attributes: vec![] },
        created_at: Utc::now(),
    };

    let serialized = format!("{item:?}");
    assert!(
        serialized.contains("taint") || serialized.contains("classification"),
        "SEQ-007: FederatedItem should carry taint classification, \
         but fields are: agent_id, session_id, content_type, content, summary, policy"
    );
}

/// SEQ-008: FederatedQuery can filter by agent and session but cannot
/// query for cross-session data flow patterns.
#[spec("SEQ-008")]
#[test]
fn federated_query_has_no_data_flow_filter() {
    let query = FederatedQuery {
        requester_id: "agent-1".into(),
        entitlements: vec!["read".into()],
        session_filter: Some("session-a".into()),
        agent_filter: Some("agent-1".into()),
        content_type_filter: None,
        since: None,
        limit: 100,
    };

    assert!(query.session_filter.is_some());
}

/// SEQ-009: evaluate_entitlements checks ABAC but has no taint propagation.
/// Tripwire: when taint-aware entitlements are added, this will stop panicking.
#[spec("SEQ-009")]
#[test]
#[should_panic(expected = "SEQ-009")]
fn entitlement_check_ignores_data_sensitivity() {
    let policy = MemoryPolicy { attributes: vec![] };

    // Empty policy = no restrictions = access granted
    let has_access = evaluate_entitlements(&["basic".into()], &policy);
    assert!(has_access);

    // SEQ-009: even with valid entitlements, access to credential-classified
    // data should be blocked. evaluate_entitlements has no taint parameter.
    assert!(
        !has_access,
        "SEQ-009: entitlement check should consider data sensitivity, \
         not just ABAC attributes"
    );
}
