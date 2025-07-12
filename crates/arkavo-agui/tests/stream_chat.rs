use arkavo_agui::types::{AgUiEvent, MessageDeltaContent};
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

#[tokio::test]
async fn test_streaming_chat_lifecycle() {
    // Create bounded channel for messages
    let (tx, mut rx) = mpsc::channel::<AgUiEvent>(32);

    // Simulate sending a user message
    let _user_message = "Hello, can you help me?";

    // Expected delta responses
    let expected_deltas = vec![
        "Hello! ",
        "I'm happy to help. ",
        "What can I assist you with today?",
    ];

    // Spawn task to simulate receiving deltas
    let message_id = uuid::Uuid::new_v4().to_string();
    let tx_clone = tx.clone();
    let expected_deltas_clone = expected_deltas.clone();

    tokio::spawn(async move {
        // Small delay to simulate network
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Send deltas
        for (i, text) in expected_deltas_clone.iter().enumerate() {
            let delta = AgUiEvent::MessageDelta {
                agent_id: "test-agent".to_string(),
                message_id: message_id.clone(),
                delta: MessageDeltaContent::Text {
                    text: text.to_string(),
                },
            };

            if tx_clone.send(delta).await.is_err() {
                break;
            }

            // Small delay between deltas
            if i < expected_deltas_clone.len() - 1 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    });

    // Collect all deltas with timeout
    let mut collected_content = String::new();
    let mut delta_count = 0;

    while delta_count < expected_deltas.len() {
        match timeout(Duration::from_secs(1), rx.recv()).await {
            Ok(Some(AgUiEvent::MessageDelta { delta, .. })) => {
                if let MessageDeltaContent::Text { text } = delta {
                    collected_content.push_str(&text);
                    delta_count += 1;
                }
            }
            Ok(Some(_)) => {
                panic!("Unexpected event type");
            }
            Ok(None) => {
                panic!("Channel closed unexpectedly");
            }
            Err(_) => {
                panic!("Timeout waiting for delta");
            }
        }
    }

    // Verify we received all expected content
    let expected_full = expected_deltas.join("");
    assert_eq!(collected_content, expected_full);
    assert_eq!(delta_count, expected_deltas.len());
}

#[tokio::test]
async fn test_subscription_cleanup_on_disconnect() {
    // Create bounded channel
    let (tx, mut rx) = mpsc::channel::<AgUiEvent>(32);

    // Drop the sender to simulate disconnect
    drop(tx);

    // Verify channel is closed
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn test_back_pressure_handling() {
    // Create small bounded channel to test back-pressure
    let (tx, mut rx) = mpsc::channel::<AgUiEvent>(2);

    // Fill the channel
    let delta1 = AgUiEvent::MessageDelta {
        agent_id: "test-agent".to_string(),
        message_id: "msg1".to_string(),
        delta: MessageDeltaContent::Text {
            text: "Delta 1".to_string(),
        },
    };

    let delta2 = AgUiEvent::MessageDelta {
        agent_id: "test-agent".to_string(),
        message_id: "msg1".to_string(),
        delta: MessageDeltaContent::Text {
            text: "Delta 2".to_string(),
        },
    };

    // These should succeed
    assert!(tx.send(delta1.clone()).await.is_ok());
    assert!(tx.send(delta2.clone()).await.is_ok());

    // This should fail with try_send (demonstrating back-pressure)
    let delta3 = AgUiEvent::MessageDelta {
        agent_id: "test-agent".to_string(),
        message_id: "msg1".to_string(),
        delta: MessageDeltaContent::Text {
            text: "Delta 3".to_string(),
        },
    };

    assert!(tx.try_send(delta3).is_err());

    // Consume one message
    let _ = rx.recv().await;

    // Now sending should work again
    let delta4 = AgUiEvent::MessageDelta {
        agent_id: "test-agent".to_string(),
        message_id: "msg1".to_string(),
        delta: MessageDeltaContent::Text {
            text: "Delta 4".to_string(),
        },
    };

    assert!(tx.send(delta4).await.is_ok());
}
