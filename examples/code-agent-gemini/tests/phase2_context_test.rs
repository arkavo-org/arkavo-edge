use arkavo_context::{CompressionPipeline, Deduplicator, SemanticChunker};

#[tokio::test]
async fn test_context_compression_pipeline() {
    let pipeline_result = CompressionPipeline::new(
        "gemma-3-4b-it".to_string(),
        Some("unsloth/gemma-3-4b-it-GGUF".to_string()),
    )
    .await;

    if pipeline_result.is_err() {
        eprintln!("⚠️  Gemma 4B model not available, skipping compression test");
        return;
    }

    let pipeline = pipeline_result.unwrap();

    let large_context = r#"
# User Authentication System

## Overview
The authentication system handles user login, registration, and session management.
It uses JWT tokens for stateless authentication and Redis for session storage.

## Components

### AuthController
Handles HTTP requests for authentication endpoints:
- POST /auth/login - User login
- POST /auth/register - New user registration
- POST /auth/logout - User logout
- GET /auth/verify - Token verification

### TokenService
Manages JWT token generation and validation:
- generateToken(user) - Creates a new JWT token
- validateToken(token) - Validates token signature
- refreshToken(token) - Generates new token from expired token

### SessionStore
Redis-based session management:
- createSession(userId, token) - Stores session
- getSession(token) - Retrieves session
- destroySession(token) - Removes session

## Security Features
- Passwords are hashed using bcrypt
- JWTs expire after 24 hours
- Refresh tokens valid for 7 days
- Rate limiting on login endpoints
- HTTPS only in production
    "#;

    let (compressed, metrics) = pipeline
        .compress(large_context, 0.6)
        .await
        .expect("Compression failed");

    println!("\n=== Context Compression Test ===");
    println!("Model: {}", pipeline.compressor_model());
    println!("Original tokens: {}", metrics.original_tokens);
    println!("Compressed tokens: {}", metrics.compressed_tokens);
    println!("Reduction: {:.1}%", metrics.reduction_percent);
    println!("Time: {}ms", metrics.compression_time_ms);
    println!("Cost saved: ${:.4}", metrics.cost_saved);
    println!("\nCompressed output length: {} chars", compressed.len());

    assert!(metrics.reduction_percent > 30.0);
    assert!(metrics.reduction_percent < 80.0);
    assert!(compressed.len() < large_context.len());
    assert!(metrics.compression_time_ms < 5000);
}

#[tokio::test]
async fn test_semantic_chunking() {
    let chunker = SemanticChunker::new(500, 50);

    let text = "First paragraph with some content.\n\n\
                Second paragraph with more content that should be separate.\n\n\
                Third paragraph that continues the document.\n\n\
                Fourth paragraph with additional information.";

    let chunks = chunker.chunk(text);

    println!("\n=== Semantic Chunking Test ===");
    println!("Input length: {} chars", text.len());
    println!("Chunks created: {}", chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        println!("Chunk {} length: {} chars", i + 1, chunk.len());
    }

    assert!(chunks.len() >= 2);
    for chunk in &chunks {
        assert!(chunk.len() <= 600);
    }
}

#[tokio::test]
async fn test_deduplication() {
    let dedup = Deduplicator::new(0.8);

    let chunks = vec![
        "This is the first unique chunk of text".to_string(),
        "This is the first unique chunk of text".to_string(),
        "This is completely different content".to_string(),
        "This is the first unique chunk of text".to_string(),
        "Another distinct piece of information".to_string(),
    ];

    let unique = dedup.deduplicate(chunks.clone());

    println!("\n=== Deduplication Test ===");
    println!("Input chunks: {}", chunks.len());
    println!("Unique chunks: {}", unique.len());
    println!("Duplicates removed: {}", chunks.len() - unique.len());

    assert_eq!(unique.len(), 3);
}

#[tokio::test]
async fn test_router_compression_flags() {
    use arkavo_router::{Router, TaskCategory};

    let router_result = Router::new().await;
    if router_result.is_err() {
        eprintln!("⚠️  Router initialization failed, skipping test");
        return;
    }

    let router = router_result.unwrap();

    let decision = router
        .classify("Create a React component with complex state management and multiple hooks")
        .await
        .expect("Routing failed");

    println!("\n=== Router Compression Decision ===");
    println!("Task category: {:?}", decision.task_category);
    println!("Model: {}", decision.recommended_model.name());
    println!("Should compress: {}", decision.should_compress);
    println!(
        "Compression target: {:?}",
        decision.compression_target.map(|t| format!("{:.0}%", t * 100.0))
    );

    assert_eq!(decision.task_category, TaskCategory::FrontendUI);
    assert!(decision.should_compress);
    assert!(decision.compression_target.is_some());
}

#[tokio::test]
async fn test_cost_savings_with_compression() {
    let original_tokens = 10000u32;
    let compressed_tokens = 3000u32;

    let metrics = arkavo_context::CompressionMetrics::new(
        original_tokens,
        compressed_tokens,
        150,
    );

    println!("\n=== Cost Savings Calculation ===");
    println!("Original tokens: {}", metrics.original_tokens);
    println!("Compressed tokens: {}", metrics.compressed_tokens);
    println!("Reduction: {:.1}%", metrics.reduction_percent);
    println!("Cost saved: ${:.4}", metrics.cost_saved);
    println!("Compression time: {}ms", metrics.compression_time_ms);

    assert_eq!(metrics.reduction_percent, 70.0);
    assert!(metrics.cost_saved > 0.0);
    assert!(metrics.cost_saved < 0.01);
}
