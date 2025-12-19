//! TØR-G Use Case Integration Tests
//!
//! Tests real-world policy scenarios using torg-core's Builder and evaluate().

use std::collections::HashMap;
use torg_core::{evaluate, Builder, Token};

/// Build a graph from a token sequence
fn build_graph(tokens: &[Token]) -> torg_core::Graph {
    let mut builder = Builder::new();
    for &token in tokens {
        builder.push(token).unwrap();
    }
    builder.finish().unwrap()
}

/// Helper to create input HashMap from boolean slice
fn inputs_map(values: &[bool]) -> HashMap<u16, bool> {
    values
        .iter()
        .enumerate()
        .map(|(i, &v)| (i as u16, v))
        .collect()
}

/// Helper to get single output from evaluation result
fn eval_output(graph: &torg_core::Graph, inputs: &[bool], output_id: u16) -> bool {
    let result = evaluate(graph, &inputs_map(inputs)).unwrap();
    result[&output_id]
}

// =============================================================================
// Test 1: Access Control (TDF)
// Policy: "Allow if Admin OR (Owner AND NOT Public)"
// =============================================================================

#[test]
fn test_access_control_tdf() {
    use Token::*;

    // Inputs: 0=admin, 1=owner, 2=public
    // NOT public = NOR(2,2) → node 3
    // NOT owner = NOR(1,1) → node 4 (for AND)
    // NOT (NOT public) = NOR(3,3) → node 5 (for AND)
    // owner AND NOT_public = NOR(4,5) → node 6
    // admin OR result = OR(0,6) → node 7
    let tokens = [
        InputDecl, Id(0), // admin
        InputDecl, Id(1), // owner
        InputDecl, Id(2), // public
        // NOT public
        NodeStart, Id(3), Nor, Id(2), Id(2), NodeEnd,
        // AND: owner AND (NOT public) = NOR(NOR(owner,owner), NOR(NOT_pub,NOT_pub))
        NodeStart, Id(4), Nor, Id(1), Id(1), NodeEnd, // NOT owner
        NodeStart, Id(5), Nor, Id(3), Id(3), NodeEnd, // NOT (NOT public)
        NodeStart, Id(6), Nor, Id(4), Id(5), NodeEnd, // AND result
        // OR: admin OR result
        NodeStart, Id(7), Or, Id(0), Id(6), NodeEnd,
        OutputDecl, Id(7),
    ];

    let graph = build_graph(&tokens);

    // admin=F, owner=T, public=F → ALLOW (owner AND NOT public = T)
    assert!(eval_output(&graph, &[false, true, false], 7));

    // admin=T, owner=F, public=T → ALLOW (admin = T)
    assert!(eval_output(&graph, &[true, false, true], 7));

    // admin=F, owner=T, public=T → DENY (owner AND NOT public = F)
    assert!(!eval_output(&graph, &[false, true, true], 7));

    // admin=F, owner=F, public=F → DENY
    assert!(!eval_output(&graph, &[false, false, false], 7));
}

// =============================================================================
// Test 2: Content Moderation Gate
// Policy: "Publish if Verified AND NOT Flagged, OR if Appeal"
// =============================================================================

#[test]
fn test_content_moderation() {
    use Token::*;

    // Inputs: 0=verified, 1=flagged, 2=appeal
    // NOT flagged = NOR(1,1) → node 3
    // verified AND NOT_flagged = NOR(NOR(0,0), NOR(3,3)) → nodes 4,5,6
    // result OR appeal = OR(6,2) → node 7
    let tokens = [
        InputDecl, Id(0), // verified
        InputDecl, Id(1), // flagged
        InputDecl, Id(2), // appeal
        // NOT flagged
        NodeStart, Id(3), Nor, Id(1), Id(1), NodeEnd,
        // AND: verified AND NOT_flagged
        NodeStart, Id(4), Nor, Id(0), Id(0), NodeEnd, // NOT verified
        NodeStart, Id(5), Nor, Id(3), Id(3), NodeEnd, // NOT (NOT flagged)
        NodeStart, Id(6), Nor, Id(4), Id(5), NodeEnd, // AND result
        // OR with appeal
        NodeStart, Id(7), Or, Id(6), Id(2), NodeEnd,
        OutputDecl, Id(7),
    ];

    let graph = build_graph(&tokens);

    // verified=T, flagged=T, appeal=F → DENY
    assert!(!eval_output(&graph, &[true, true, false], 7));

    // verified=T, flagged=F, appeal=F → PUBLISH
    assert!(eval_output(&graph, &[true, false, false], 7));

    // verified=F, flagged=T, appeal=T → PUBLISH (appeal overrides)
    assert!(eval_output(&graph, &[false, true, true], 7));
}

// =============================================================================
// Test 3: Agent Routing (Mesh)
// Policy: "Route to agent if Capable AND (Local XOR NOT Busy)"
// =============================================================================

#[test]
fn test_agent_routing_mesh() {
    use Token::*;

    // Inputs: 0=capable, 1=local, 2=busy
    // NOT busy = NOR(2,2) → node 3
    // local XOR NOT_busy = XOR(1,3) → node 4
    // capable AND xor_result = NOR(NOR(0,0), NOR(4,4)) → nodes 5,6,7
    let tokens = [
        InputDecl, Id(0), // capable
        InputDecl, Id(1), // local
        InputDecl, Id(2), // busy
        // NOT busy
        NodeStart, Id(3), Nor, Id(2), Id(2), NodeEnd,
        // XOR: local XOR NOT_busy
        NodeStart, Id(4), Xor, Id(1), Id(3), NodeEnd,
        // AND: capable AND xor_result
        NodeStart, Id(5), Nor, Id(0), Id(0), NodeEnd, // NOT capable
        NodeStart, Id(6), Nor, Id(4), Id(4), NodeEnd, // NOT xor_result
        NodeStart, Id(7), Nor, Id(5), Id(6), NodeEnd, // AND result
        OutputDecl, Id(7),
    ];

    let graph = build_graph(&tokens);

    // capable=T, local=F, busy=F → ROUTE
    // NOT_busy=T, local XOR NOT_busy = F XOR T = T, capable AND T = T
    assert!(eval_output(&graph, &[true, false, false], 7));

    // capable=T, local=T, busy=T → ROUTE
    // NOT_busy=F, local XOR NOT_busy = T XOR F = T, capable AND T = T
    assert!(eval_output(&graph, &[true, true, true], 7));

    // capable=T, local=T, busy=F → NO ROUTE
    // NOT_busy=T, local XOR NOT_busy = T XOR T = F, capable AND F = F
    assert!(!eval_output(&graph, &[true, true, false], 7));

    // capable=F, local=F, busy=F → NO ROUTE (not capable)
    assert!(!eval_output(&graph, &[false, false, false], 7));
}

// =============================================================================
// Test 4: Feature Flag Evaluation
// Policy: "Show feature if Enabled OR (Beta XOR Staff)"
// =============================================================================

#[test]
fn test_feature_flags() {
    use Token::*;

    // Inputs: 0=beta, 1=staff, 2=enabled
    // beta XOR staff = XOR(0,1) → node 3
    // enabled OR xor_result = OR(2,3) → node 4
    let tokens = [
        InputDecl, Id(0), // beta
        InputDecl, Id(1), // staff
        InputDecl, Id(2), // enabled
        // XOR: beta XOR staff
        NodeStart, Id(3), Xor, Id(0), Id(1), NodeEnd,
        // OR: enabled OR xor_result
        NodeStart, Id(4), Or, Id(2), Id(3), NodeEnd,
        OutputDecl, Id(4),
    ];

    let graph = build_graph(&tokens);

    // beta=T, staff=T, enabled=F → HIDE (XOR=false, OR=false)
    assert!(!eval_output(&graph, &[true, true, false], 4));

    // beta=T, staff=F, enabled=F → SHOW (XOR=true)
    assert!(eval_output(&graph, &[true, false, false], 4));

    // beta=F, staff=F, enabled=T → SHOW (enabled=true)
    assert!(eval_output(&graph, &[false, false, true], 4));
}

// =============================================================================
// Test 5: Rate Limit Bypass
// Policy: "Bypass limit if (Premium OR Partner) AND Throttled"
// =============================================================================

#[test]
fn test_rate_limit_bypass() {
    use Token::*;

    // Inputs: 0=premium, 1=partner, 2=throttled
    // premium OR partner = OR(0,1) → node 3
    // or_result AND throttled = NOR(NOR(3,3), NOR(2,2)) → nodes 4,5,6
    let tokens = [
        InputDecl, Id(0), // premium
        InputDecl, Id(1), // partner
        InputDecl, Id(2), // throttled
        // OR: premium OR partner
        NodeStart, Id(3), Or, Id(0), Id(1), NodeEnd,
        // AND: or_result AND throttled
        NodeStart, Id(4), Nor, Id(3), Id(3), NodeEnd, // NOT or_result
        NodeStart, Id(5), Nor, Id(2), Id(2), NodeEnd, // NOT throttled
        NodeStart, Id(6), Nor, Id(4), Id(5), NodeEnd, // AND result
        OutputDecl, Id(6),
    ];

    let graph = build_graph(&tokens);

    // premium=F, partner=T, throttled=T → BYPASS
    assert!(eval_output(&graph, &[false, true, true], 6));

    // premium=T, partner=F, throttled=F → NO BYPASS (not throttled)
    assert!(!eval_output(&graph, &[true, false, false], 6));

    // premium=F, partner=F, throttled=T → NO BYPASS (not premium or partner)
    assert!(!eval_output(&graph, &[false, false, true], 6));
}

// =============================================================================
// Test 6: Multi-Party Approval (Majority Vote)
// Policy: "Execute if at least 2 of 3 approve"
// Formula: (A AND B) OR (B AND C) OR (A AND C)
// =============================================================================

#[test]
fn test_multi_party_approval() {
    use Token::*;

    // Inputs: 0=alice, 1=bob, 2=charlie
    // A AND B = NOR(NOR(0,0), NOR(1,1)) → nodes 3,4,5
    // B AND C = NOR(NOR(1,1), NOR(2,2)) → nodes 6,7,8
    // A AND C = NOR(NOR(0,0), NOR(2,2)) → nodes 9,10,11
    // (A&B) OR (B&C) = OR(5,8) → node 12
    // result OR (A&C) = OR(12,11) → node 13
    let tokens = [
        InputDecl, Id(0), // alice
        InputDecl, Id(1), // bob
        InputDecl, Id(2), // charlie
        // A AND B
        NodeStart, Id(3), Nor, Id(0), Id(0), NodeEnd, // NOT alice
        NodeStart, Id(4), Nor, Id(1), Id(1), NodeEnd, // NOT bob
        NodeStart, Id(5), Nor, Id(3), Id(4), NodeEnd, // alice AND bob
        // B AND C
        NodeStart, Id(6), Nor, Id(1), Id(1), NodeEnd, // NOT bob (reuse would be nice)
        NodeStart, Id(7), Nor, Id(2), Id(2), NodeEnd, // NOT charlie
        NodeStart, Id(8), Nor, Id(6), Id(7), NodeEnd, // bob AND charlie
        // A AND C
        NodeStart, Id(9), Nor, Id(0), Id(0), NodeEnd,  // NOT alice
        NodeStart, Id(10), Nor, Id(2), Id(2), NodeEnd, // NOT charlie
        NodeStart, Id(11), Nor, Id(9), Id(10), NodeEnd, // alice AND charlie
        // Combine with ORs
        NodeStart, Id(12), Or, Id(5), Id(8), NodeEnd,   // (A&B) OR (B&C)
        NodeStart, Id(13), Or, Id(12), Id(11), NodeEnd, // final OR (A&C)
        OutputDecl, Id(13),
    ];

    let graph = build_graph(&tokens);

    // alice=T, bob=F, charlie=T → EXECUTE (alice AND charlie)
    assert!(eval_output(&graph, &[true, false, true], 13));

    // alice=T, bob=T, charlie=F → EXECUTE (alice AND bob)
    assert!(eval_output(&graph, &[true, true, false], 13));

    // alice=F, bob=T, charlie=T → EXECUTE (bob AND charlie)
    assert!(eval_output(&graph, &[false, true, true], 13));

    // alice=T, bob=T, charlie=T → EXECUTE (all approve)
    assert!(eval_output(&graph, &[true, true, true], 13));

    // alice=T, bob=F, charlie=F → NO EXECUTE (only 1)
    assert!(!eval_output(&graph, &[true, false, false], 13));

    // alice=F, bob=F, charlie=F → NO EXECUTE (none)
    assert!(!eval_output(&graph, &[false, false, false], 13));
}
