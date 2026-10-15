use crate::app::App;
use crate::{LlmRequest, LlmResponse};
use arkavo_test_macros::spec;

#[test]
#[spec("TERM-001")]
fn app_new_initializes_views_and_event_state() {
    let app = App::new();

    // Event loop starts in a running state with shortcuts registered
    assert!(!app.should_quit);
    assert!(!app.event_handler.shortcuts.is_empty());
    // Views render from the start (chat view shows its welcome message)
    assert!(!app.chat_view.messages.is_empty());
    assert!(!app.thinking_view.messages.is_empty());
    assert!(!app.providers.is_empty());
    // Channels are only wired once the LLM runtime is available
    assert!(app.ui_tx.is_none());
    assert!(app.llm_rx.is_none());
}

#[test]
#[spec("TERM-001")]
fn app_with_channels_wires_llm_channels() {
    let (ui_tx, mut ui_rx) = tokio::sync::mpsc::channel::<LlmRequest>(100);
    let (_llm_tx, llm_rx) = tokio::sync::mpsc::channel::<LlmResponse>(100);
    let app = App::new_with_channels(ui_tx, llm_rx);

    // The UI half of the channel pair is live and carries requests
    let request = LlmRequest {
        task_id: uuid::Uuid::new_v4(),
        model_name: "test-model".to_string(),
        prompt: "hello".to_string(),
    };
    let task_id = request.task_id;
    app.ui_tx.as_ref().unwrap().try_send(request).unwrap();

    let received = ui_rx.try_recv().unwrap();
    assert_eq!(received.task_id, task_id);
    assert_eq!(received.model_name, "test-model");
    assert_eq!(received.prompt, "hello");
}
