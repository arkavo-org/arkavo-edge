use arkavo_memory::embeddings::EmbeddingService;

#[tokio::test]
async fn test_embedding_generation() {
    let service = EmbeddingService::new();
    
    // Test embedding generation
    let text = "This is a test sentence for embeddings";
    let embedding = service.generate_embedding(text).await.unwrap();
    
    // AllMiniLML6V2 produces 384-dimensional embeddings
    assert_eq!(embedding.len(), 384);
    
    // Check that values are normalized (roughly)
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 0.1, "Embedding should be approximately normalized");
    
    // Check that values are in a reasonable range
    for value in &embedding {
        assert!(value.abs() < 10.0, "Embedding values should be reasonable");
    }
}

#[tokio::test]
async fn test_embedding_consistency() {
    let service = EmbeddingService::new();
    
    // Same text should produce same embedding
    let text = "Consistent embedding test";
    let embedding1 = service.generate_embedding(text).await.unwrap();
    let embedding2 = service.generate_embedding(text).await.unwrap();
    
    // Check embeddings are identical
    for (a, b) in embedding1.iter().zip(embedding2.iter()) {
        assert!((a - b).abs() < 1e-6, "Same text should produce identical embeddings");
    }
}

#[tokio::test]
async fn test_cosine_similarity() {
    let service = EmbeddingService::new();
    
    // Generate embeddings for similar sentences
    let text1 = "The cat sat on the mat";
    let text2 = "The feline rested on the rug";
    let text3 = "Python is a programming language";
    
    let embedding1 = service.generate_embedding(text1).await.unwrap();
    let embedding2 = service.generate_embedding(text2).await.unwrap();
    let embedding3 = service.generate_embedding(text3).await.unwrap();
    
    // Similar sentences should have high similarity
    let similarity_1_2 = EmbeddingService::cosine_similarity(&embedding1, &embedding2);
    let similarity_1_3 = EmbeddingService::cosine_similarity(&embedding1, &embedding3);
    
    println!("Similarity between similar sentences: {}", similarity_1_2);
    println!("Similarity between different sentences: {}", similarity_1_3);
    
    // Similar sentences should have higher similarity than different ones
    assert!(similarity_1_2 > similarity_1_3, 
        "Similar sentences should have higher cosine similarity");
    
    // Self-similarity should be ~1.0
    let self_similarity = EmbeddingService::cosine_similarity(&embedding1, &embedding1);
    assert!((self_similarity - 1.0).abs() < 1e-6, "Self-similarity should be 1.0");
}

#[tokio::test]
async fn test_embedding_dimension() {
    let service = EmbeddingService::new();
    
    // Check that embedding_dimension matches actual embedding size
    let expected_dim = service.embedding_dimension();
    let embedding = service.generate_embedding("test").await.unwrap();
    
    assert_eq!(embedding.len(), expected_dim, 
        "Embedding dimension should match expected value");
}