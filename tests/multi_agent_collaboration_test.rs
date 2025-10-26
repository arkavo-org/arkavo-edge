use arkavo_memory::{AgentConversation, MemoryStorage};
use arkavo_protocol::{
    A2aServer, A2aTransport, AgentBroadcast, AgentQueryRequest,
    AgentQueryResponse, BroadcastType, WebSocketTransport,
};
use chrono::Utc;
use tokio::time::sleep;
use tracing::{error, info};
use uuid::Uuid;

/// Test demonstrating multi-agent collaboration
/// 
/// This test simulates a scenario where:
/// 1. A user asks the orchestrator to review a Python web app for security
/// 2. The orchestrator broadcasts a task and queries specialized agents
/// 3. Multiple agents collaborate to provide a comprehensive security review
#[tokio::test]
async fn test_multi_agent_collaboration() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("Starting multi-agent collaboration test");

    // Initialize shared memory storage
    let memory = MemoryStorage::new_test().await?;

    // Simulate agent startup and capability broadcasts
    let agents = vec![
        ("orchestrator-agent", "8342", vec!["orchestration", "task_decomposition"]),
        ("security-agent", "8343", vec!["security_analysis", "vulnerability_detection"]),
        ("code-review-agent", "8344", vec!["code_review", "pattern_analysis"]),
        ("database-agent", "8345", vec!["database_optimization", "sql_analysis"]),
    ];

    // Simulate capability broadcasts
    for (agent_id, _port, capabilities) in &agents {
        let broadcast = AgentBroadcast {
            agent_id: agent_id.to_string(),
            broadcast_type: BroadcastType::Available,
            capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
            metadata: None,
        };
        
        info!("Agent {} broadcasting capabilities: {:?}", agent_id, capabilities);
    }

    // Simulate user request to orchestrator
    let user_query = "Review my Python web app for security issues";
    info!("User query: {}", user_query);

    // Orchestrator decomposes the task
    let orchestrator_analysis = vec![
        "1. Security analysis needed",
        "2. Code review for patterns",
        "3. Database query analysis",
    ];
    info!("Orchestrator task decomposition: {:?}", orchestrator_analysis);

    // Orchestrator queries security agent
    let security_query = AgentQueryRequest {
        from_agent_id: "orchestrator-agent".to_string(),
        to_agent_id: Some("security-agent".to_string()),
        query: "Analyze Python web app code for security vulnerabilities".to_string(),
        context: None,
        domain: Some("security".to_string()),
    };

    // Security agent analyzes and finds issue
    let security_response = AgentQueryResponse {
        from_agent_id: "security-agent".to_string(),
        response: "Found potential SQL injection vulnerability in user_login function".to_string(),
        confidence: 0.85,
        domain: Some("security".to_string()),
        evidence: Some(serde_json::json!({
            "file": "auth.py",
            "line": 42,
            "code": "query = f\"SELECT * FROM users WHERE username='{username}'\""
        })),
    };

    // Store conversation in memory
    let conversation1 = AgentConversation {
        id: Uuid::new_v4(),
        from_agent_id: security_query.from_agent_id.clone(),
        to_agent_id: security_query.to_agent_id.clone().unwrap(),
        query: security_query.query.clone(),
        response: security_response.response.clone(),
        confidence: security_response.confidence,
        domain: security_response.domain.clone(),
        created_at: Utc::now(),
    };
    memory.store_conversation(conversation1).await?;

    // Security agent queries code review agent for confirmation
    let code_review_query = AgentQueryRequest {
        from_agent_id: "security-agent".to_string(),
        to_agent_id: Some("code-review-agent".to_string()),
        query: "Is this SQL query using parameterized statements: SELECT * FROM users WHERE username='{username}'".to_string(),
        context: Some(serde_json::json!({"file": "auth.py", "line": 42})),
        domain: Some("code_review".to_string()),
    };

    let code_review_response = AgentQueryResponse {
        from_agent_id: "code-review-agent".to_string(),
        response: "No, this uses string concatenation which is vulnerable to SQL injection. Should use parameterized queries.".to_string(),
        confidence: 0.95,
        domain: Some("code_review".to_string()),
        evidence: None,
    };

    // Store this conversation too
    let conversation2 = AgentConversation {
        id: Uuid::new_v4(),
        from_agent_id: code_review_query.from_agent_id.clone(),
        to_agent_id: code_review_query.to_agent_id.clone().unwrap(),
        query: code_review_query.query.clone(),
        response: code_review_response.response.clone(),
        confidence: code_review_response.confidence,
        domain: code_review_response.domain.clone(),
        created_at: Utc::now(),
    };
    memory.store_conversation(conversation2).await?;

    // Orchestrator aggregates responses
    let final_report = format!(
        "Security Review Report:\n\n\
        1. CRITICAL: SQL Injection Vulnerability\n\
        - Location: auth.py, line 42\n\
        - Issue: Using string concatenation in SQL query\n\
        - Recommendation: Use parameterized queries\n\
        - Confidence: {}%\n\n\
        Verified by: code-review-agent ({}% confidence)",
        (security_response.confidence * 100.0) as u8,
        (code_review_response.confidence * 100.0) as u8
    );

    info!("Final report from orchestrator:\n{}", final_report);

    // Verify conversations were stored
    let stored_conversations = memory.get_agent_conversations(
        Some("orchestrator-agent"),
        None,
        None,
        10
    ).await?;
    
    assert!(!stored_conversations.is_empty(), "Should have stored conversations");
    info!("Stored {} agent conversations", stored_conversations.len());

    // Test domain-specific search
    let security_knowledge = memory.search_by_domain(
        "security",
        "SQL injection",
        5
    ).await?;
    
    info!("Found {} security-related memories", security_knowledge.len());

    Ok(())
}

/// Test agent discovery via mDNS simulation
#[tokio::test]
async fn test_agent_discovery() -> anyhow::Result<()> {
    info!("Testing agent discovery");

    // Simulate mDNS discovery results
    let discovered_agents = vec![
        ("orchestrator-agent", "192.168.1.100:8342", vec!["orchestration"]),
        ("security-agent", "192.168.1.101:8343", vec!["security_analysis"]),
        ("code-review-agent", "192.168.1.102:8344", vec!["code_review"]),
    ];

    for (agent_id, endpoint, capabilities) in discovered_agents {
        info!(
            "Discovered agent: {} at {} with capabilities: {:?}",
            agent_id, endpoint, capabilities
        );
    }

    Ok(())
}

/// Test agent query routing
#[tokio::test]
async fn test_agent_query_routing() -> anyhow::Result<()> {
    info!("Testing agent query routing");

    // Test broadcast query (no specific target)
    let broadcast_query = AgentQueryRequest {
        from_agent_id: "orchestrator-agent".to_string(),
        to_agent_id: None, // Broadcast to all
        query: "Who can analyze SQL queries?".to_string(),
        context: None,
        domain: Some("database".to_string()),
    };

    // Simulate responses from multiple agents
    let responses = vec![
        AgentQueryResponse {
            from_agent_id: "database-agent".to_string(),
            response: "I specialize in SQL query optimization and analysis".to_string(),
            confidence: 0.95,
            domain: Some("database".to_string()),
            evidence: None,
        },
        AgentQueryResponse {
            from_agent_id: "security-agent".to_string(),
            response: "I can analyze SQL queries for injection vulnerabilities".to_string(),
            confidence: 0.8,
            domain: Some("security".to_string()),
            evidence: None,
        },
    ];

    info!("Broadcast query: {}", broadcast_query.query);
    for response in responses {
        info!(
            "Response from {}: {} (confidence: {})",
            response.from_agent_id, response.response, response.confidence
        );
    }

    Ok(())
}