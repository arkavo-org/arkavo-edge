#[cfg(test)]
mod llm_integration_tests {
    use arkavo_terminal::{LlmRequest, LlmResponse};
    use tokio::sync::mpsc;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_llm_request_response_flow() {
        // Create channels
        let (ui_tx, mut ui_rx) = mpsc::channel::<LlmRequest>(10);
        let (llm_tx, _llm_rx) = mpsc::channel::<LlmResponse>(10);

        // Simulate sending a request
        let task_id = Uuid::new_v4();
        let request = LlmRequest {
            task_id,
            model_name: "test-model".to_string(),
            prompt: "Hello, world!".to_string(),
        };

        ui_tx.send(request.clone()).await.unwrap();

        // Verify request was received
        let received_request = ui_rx.recv().await.unwrap();
        assert_eq!(received_request.task_id, task_id);
        assert_eq!(received_request.model_name, "test-model");
        assert_eq!(received_request.prompt, "Hello, world!");

        // Simulate streaming response
        let response1 = LlmResponse {
            task_id,
            model_name: "test-model".to_string(),
            content: "Hello".to_string(),
            is_streaming: true,
            is_complete: false,
            error: None,
        };
        llm_tx.send(response1).await.unwrap();

        let response2 = LlmResponse {
            task_id,
            model_name: "test-model".to_string(),
            content: " there!".to_string(),
            is_streaming: true,
            is_complete: false,
            error: None,
        };
        llm_tx.send(response2).await.unwrap();

        let response3 = LlmResponse {
            task_id,
            model_name: "test-model".to_string(),
            content: "".to_string(),
            is_streaming: true,
            is_complete: true,
            error: None,
        };
        llm_tx.send(response3).await.unwrap();

        // Drop channels to clean up
        drop(ui_tx);
        drop(llm_tx);
    }

    #[test]
    fn test_multiple_model_support() {
        let models = vec![
            "llava:7b".to_string(),
            "devstral:latest".to_string(),
            "deepseek-r1:14b".to_string(),
            "qwen3:0.6b".to_string(),
            "dolphin3:latest".to_string(),
        ];

        // Verify we can create requests for different models
        for model in models {
            let task_id = Uuid::new_v4();
            let request = LlmRequest {
                task_id,
                model_name: model.clone(),
                prompt: format!("Test prompt for {model}"),
            };

            assert_eq!(request.model_name, model);
        }
    }
}
