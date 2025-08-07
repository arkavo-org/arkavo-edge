// Core A2A protocol integration tests
use arkavo_protocol::{
    file_transfer::{FileTransferManager, FileUploadRequest, FileMetadata, FileChunk, calculate_chunks},
    oauth2::{OAuth2Provider, OAuth2Config, TokenRequest, GrantType},
};
use tempfile::TempDir;
use uuid::Uuid;

#[tokio::test]
async fn test_oauth2_authorization_code_flow() {
    let config = OAuth2Config {
        client_id: "test_client".to_string(),
        client_secret: "test_secret".to_string(),
        authorization_endpoint: "https://auth.example.com/authorize".to_string(),
        token_endpoint: "https://auth.example.com/token".to_string(),
        redirect_uri: "https://app.example.com/callback".to_string(),
        scopes: vec!["read".to_string(), "write".to_string()],
        issuer: "https://auth.example.com".to_string(),
        audience: Some("https://api.example.com".to_string()),
    };

    let provider = OAuth2Provider::new(config, "jwt_secret_key".to_string());

    // Generate authorization code
    let code = provider
        .generate_authorization_code(
            "test_client".to_string(),
            "https://app.example.com/callback".to_string(),
            vec!["read".to_string()],
            "user123".to_string(),
        )
        .await
        .unwrap();

    // Exchange code for tokens
    let token_request = TokenRequest {
        grant_type: GrantType::AuthorizationCode,
        code: Some(code),
        refresh_token: None,
        client_id: "test_client".to_string(),
        client_secret: Some("test_secret".to_string()),
        redirect_uri: Some("https://app.example.com/callback".to_string()),
        scope: None,
    };

    let response = provider
        .exchange_authorization_code(token_request)
        .await
        .unwrap();

    assert_eq!(response.token_type, "Bearer");
    assert!(response.refresh_token.is_some());
    assert_eq!(response.expires_in, 3600);
}

#[tokio::test]
async fn test_file_transfer_chunked_upload() {
    let temp_dir = TempDir::new().unwrap();
    let manager = FileTransferManager::new(temp_dir.path()).unwrap();
    manager.initialize().await.unwrap();

    let file_id = Uuid::new_v4().to_string();
    let file_content = b"Test file content for A2A protocol";
    let chunk_size = 10;
    let chunks_total = calculate_chunks(file_content.len() as u64, chunk_size);

    // Upload file in chunks
    for (i, chunk_data) in file_content.chunks(chunk_size).enumerate() {
        let metadata = FileMetadata {
            id: file_id.clone(),
            name: "test.txt".to_string(),
            size: file_content.len() as u64,
            mime_type: Some("text/plain".to_string()),
            checksum: None,
            created_at: chrono::Utc::now().timestamp(),
            chunks_total,
            chunks_received: 0,
        };

        let chunk = FileChunk {
            file_id: file_id.clone(),
            chunk_index: i,
            data: chunk_data.to_vec(),
            is_last: i == chunks_total - 1,
        };

        let request = FileUploadRequest { metadata, chunk };
        let response = manager.handle_upload(request).await.unwrap();

        // Verify response
        assert_eq!(response.file_id, file_id);
        assert_eq!(response.chunks_received, i + 1);
        assert_eq!(response.chunks_total, chunks_total);
        
        if i == chunks_total - 1 {
            assert!(response.completed);
        } else {
            assert!(!response.completed);
        }
    }

    // Verify file was saved
    let files = manager.list_files().await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, file_id);
}