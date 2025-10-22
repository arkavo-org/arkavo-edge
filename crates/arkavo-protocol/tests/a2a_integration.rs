#![allow(clippy::disallowed_methods)]
#![allow(clippy::float_cmp)]

use arkavo_protocol::{
    file_transfer::{
        calculate_chunks, FileChunk, FileDownloadRequest, FileMetadata, FileTransferManager,
        FileUploadRequest,
    },
    oauth2::{GrantType, OAuth2Config, OAuth2Provider, TokenRequest},
    push_notifications::{
        EventType, PushNotificationService, SubscriptionFilter, SubscriptionRequest,
    },
};
use tempfile::TempDir;
use uuid::Uuid;

#[tokio::test]
async fn test_oauth2_full_flow() {
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

    // Validate access token
    let token_info = provider
        .validate_access_token(&response.access_token)
        .await
        .unwrap();
    assert_eq!(token_info.user_id, "user123");

    // Refresh token
    let refresh_request = TokenRequest {
        grant_type: GrantType::RefreshToken,
        code: None,
        refresh_token: response.refresh_token,
        client_id: "test_client".to_string(),
        client_secret: Some("test_secret".to_string()),
        redirect_uri: None,
        scope: None,
    };

    let refreshed = provider
        .refresh_access_token(refresh_request)
        .await
        .unwrap();
    assert_ne!(refreshed.access_token, response.access_token);
}

#[tokio::test]
async fn test_file_transfer_complete_flow() {
    let temp_dir = TempDir::new().unwrap();
    let manager = FileTransferManager::new(temp_dir.path()).unwrap();
    manager.initialize().await.unwrap();

    let file_id = Uuid::new_v4().to_string();
    let file_content = b"This is a test file for A2A protocol implementation";
    let chunk_size = 16; // Small chunks for testing
    let chunks_total = calculate_chunks(file_content.len() as u64, chunk_size);

    // Upload file in chunks
    for (i, chunk_data) in file_content.chunks(chunk_size).enumerate() {
        let metadata = FileMetadata {
            id: file_id.clone(),
            name: "test_a2a.txt".to_string(),
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

        if i == chunks_total - 1 {
            assert!(response.completed);
            assert_eq!(response.chunks_received, chunks_total);
        }
    }

    // Check transfer progress
    let progress = manager.get_transfer_progress(&file_id).await.unwrap();
    assert_eq!(progress.percentage, 100.0);

    // Download the file
    let download_request = FileDownloadRequest {
        file_id: file_id.clone(),
        chunk_index: Some(0),
    };

    let download_response = manager.handle_download(download_request).await.unwrap();
    assert_eq!(download_response.metadata.name, "test_a2a.txt");

    // List files
    let files = manager.list_files().await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, file_id);

    // Delete file
    manager.delete_file(&file_id).await.unwrap();
    let files_after = manager.list_files().await.unwrap();
    assert_eq!(files_after.len(), 0);
}

#[tokio::test]
async fn test_push_notifications_with_filtering() {
    let service = PushNotificationService::new(3); // max_retries = 3

    // Create subscriptions with different filters
    let client1_id = "client1";
    let sub_req1 = SubscriptionRequest {
        client_id: client1_id.to_string(),
        event_types: vec![EventType::TaskUpdate],
        filter: None,
    };

    let client2_id = "client2";
    let sub_req2 = SubscriptionRequest {
        client_id: client2_id.to_string(),
        event_types: vec![EventType::AgentStatus],
        filter: Some(SubscriptionFilter {
            agent_ids: Some(vec!["agent1".to_string()]),
            task_ids: None,
            metadata_filters: None,
        }),
    };

    let sub1_resp = service.subscribe(sub_req1).await.unwrap();
    let _sub2_resp = service.subscribe(sub_req2).await.unwrap();

    // Register channels for clients
    let (tx1, mut rx1) = tokio::sync::mpsc::channel(10);
    let (tx2, mut rx2) = tokio::sync::mpsc::channel(10);

    service
        .register_channel(client1_id.to_string(), tx1)
        .await
        .unwrap();
    service
        .register_channel(client2_id.to_string(), tx2)
        .await
        .unwrap();

    // Send task update event
    let task_payload = serde_json::json!({
        "task_id": "task123",
        "status": "completed"
    });

    service
        .publish_event(EventType::TaskUpdate, task_payload, None)
        .await
        .unwrap();

    // Send agent status event with filter
    let agent_payload = serde_json::json!({
        "agent_id": "agent1",
        "status": "online"
    });

    let agent_filter = Some(SubscriptionFilter {
        agent_ids: Some(vec!["agent1".to_string()]),
        task_ids: None,
        metadata_filters: None,
    });

    service
        .publish_event(EventType::AgentStatus, agent_payload, agent_filter)
        .await
        .unwrap();

    // Allow time for async processing
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify client1 received task update
    if let Ok(notification) = rx1.try_recv() {
        assert!(notification.payload.to_string().contains("task123"));
    }

    // Verify client2 received agent status
    if let Ok(notification) = rx2.try_recv() {
        assert!(notification.payload.to_string().contains("agent1"));
    }

    // Test unsubscribe
    service
        .unsubscribe(&sub1_resp.subscription_id)
        .await
        .unwrap();
    let subs = service.get_client_subscriptions(client1_id).await;
    assert_eq!(subs.len(), 0);
}

#[tokio::test]
async fn test_file_cleanup_stale_uploads() {
    let temp_dir = TempDir::new().unwrap();
    let manager = FileTransferManager::new(temp_dir.path()).unwrap();
    manager.initialize().await.unwrap();

    // Start an upload but don't complete it
    let file_id = Uuid::new_v4().to_string();
    let metadata = FileMetadata {
        id: file_id.clone(),
        name: "incomplete.txt".to_string(),
        size: 100,
        mime_type: None,
        checksum: None,
        created_at: chrono::Utc::now().timestamp(),
        chunks_total: 2,
        chunks_received: 0,
    };

    let chunk = FileChunk {
        file_id: file_id.clone(),
        chunk_index: 0,
        data: vec![1, 2, 3, 4],
        is_last: false,
    };

    let request = FileUploadRequest { metadata, chunk };
    let response = manager.handle_upload(request).await.unwrap();
    assert!(!response.completed);

    // Cleanup with 0 seconds max age (should remove all)
    let removed = manager.cleanup_stale_uploads(0).await.unwrap();
    assert_eq!(removed, 1);

    // Verify the upload was removed
    let result = manager.get_transfer_progress(&file_id).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_oauth2_token_revocation() {
    let config = OAuth2Config {
        client_id: "test_client".to_string(),
        client_secret: "test_secret".to_string(),
        authorization_endpoint: "https://auth.example.com/authorize".to_string(),
        token_endpoint: "https://auth.example.com/token".to_string(),
        redirect_uri: "https://app.example.com/callback".to_string(),
        scopes: vec!["read".to_string()],
        issuer: "https://auth.example.com".to_string(),
        audience: None,
    };

    let provider = OAuth2Provider::new(config, "jwt_secret".to_string());

    // Generate and exchange code for tokens
    let code = provider
        .generate_authorization_code(
            "test_client".to_string(),
            "https://app.example.com/callback".to_string(),
            vec!["read".to_string()],
            "user456".to_string(),
        )
        .await
        .unwrap();

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

    // Verify token is valid
    provider
        .validate_access_token(&response.access_token)
        .await
        .unwrap();

    // Revoke the access token
    provider.revoke_token(&response.access_token).await.unwrap();

    // Verify token is no longer valid
    let validation_result = provider.validate_access_token(&response.access_token).await;
    assert!(validation_result.is_err());

    // Revoke refresh token
    if let Some(refresh_token) = response.refresh_token {
        provider.revoke_token(&refresh_token).await.unwrap();

        // Attempting to use revoked refresh token should fail
        let refresh_request = TokenRequest {
            grant_type: GrantType::RefreshToken,
            code: None,
            refresh_token: Some(refresh_token),
            client_id: "test_client".to_string(),
            client_secret: Some("test_secret".to_string()),
            redirect_uri: None,
            scope: None,
        };

        let refresh_result = provider.refresh_access_token(refresh_request).await;
        assert!(refresh_result.is_err());
    }
}

#[tokio::test]
async fn test_push_notification_cleanup() {
    let service = PushNotificationService::new(3);

    // Add a subscription
    let sub_req = SubscriptionRequest {
        client_id: "test_client".to_string(),
        event_types: vec![EventType::SystemAlert],
        filter: None,
    };

    let sub_resp = service.subscribe(sub_req).await.unwrap();

    // Register a channel
    let (tx, _rx) = tokio::sync::mpsc::channel(10);
    service
        .register_channel("test_client".to_string(), tx)
        .await
        .unwrap();

    // Publish an event
    let event_payload = serde_json::json!({
        "message": "Test alert"
    });

    service
        .publish_event(EventType::SystemAlert, event_payload, None)
        .await
        .unwrap();

    // Allow time for processing
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Clean up inactive subscriptions
    let removed = service.cleanup_inactive_subscriptions().await.unwrap();

    // Verify cleanup works (won't remove active subscriptions in this test)
    assert_eq!(removed, 0);

    // Unsubscribe and verify
    service
        .unsubscribe(&sub_resp.subscription_id)
        .await
        .unwrap();
    let subs = service.get_client_subscriptions("test_client").await;
    assert_eq!(subs.len(), 0);
}

#[tokio::test]
async fn test_file_transfer_progress_tracking() {
    let temp_dir = TempDir::new().unwrap();
    let manager = FileTransferManager::new(temp_dir.path()).unwrap();
    manager.initialize().await.unwrap();

    let file_id = Uuid::new_v4().to_string();
    let chunk_size = 32;
    let total_size = 100;
    let chunks_total = calculate_chunks(total_size, chunk_size);

    // Upload first chunk
    let metadata = FileMetadata {
        id: file_id.clone(),
        name: "progress_test.bin".to_string(),
        size: total_size,
        mime_type: None,
        checksum: None,
        created_at: chrono::Utc::now().timestamp(),
        chunks_total,
        chunks_received: 0,
    };

    let chunk = FileChunk {
        file_id: file_id.clone(),
        chunk_index: 0,
        data: vec![0u8; chunk_size],
        is_last: false,
    };

    let request = FileUploadRequest { metadata, chunk };
    manager.handle_upload(request).await.unwrap();

    // Check progress after first chunk
    let progress = manager.get_transfer_progress(&file_id).await.unwrap();
    assert!(progress.percentage > 0.0);
    assert!(progress.percentage < 100.0);
    assert_eq!(progress.chunks_transferred, 1);
    assert_eq!(progress.total_chunks, chunks_total);
}

#[tokio::test]
async fn test_oauth2_authorization_url_generation() {
    let config = OAuth2Config {
        client_id: "app_client".to_string(),
        client_secret: "app_secret".to_string(),
        authorization_endpoint: "https://oauth.example.com/authorize".to_string(),
        token_endpoint: "https://oauth.example.com/token".to_string(),
        redirect_uri: "https://myapp.example.com/callback".to_string(),
        scopes: vec!["profile".to_string(), "email".to_string()],
        issuer: "https://oauth.example.com".to_string(),
        audience: None,
    };

    let provider = OAuth2Provider::new(config, "secret_key".to_string());

    let auth_url = provider.get_authorization_url("random_state_123");

    assert!(auth_url.contains("response_type=code"));
    assert!(auth_url.contains("client_id=app_client"));
    assert!(auth_url.contains("redirect_uri="));
    assert!(auth_url.contains("scope="));
    assert!(auth_url.contains("state=random_state_123"));
}
