use crate::ui::chat::{ChatView, MessageRole};

#[test]
fn test_chat_message_storage() {
    let mut chat = ChatView::new();

    // ChatView now starts with a welcome message
    assert_eq!(chat.messages.len(), 1);
    assert_eq!(chat.messages[0].role, MessageRole::System);

    // Add some messages
    chat.add_message(MessageRole::User, "Hello".to_string());
    chat.add_message(MessageRole::Assistant, "Hi there!".to_string());

    assert_eq!(chat.messages.len(), 3);
    assert_eq!(chat.messages[1].role, MessageRole::User);
    assert_eq!(chat.messages[1].content, "Hello");
    assert_eq!(chat.messages[2].role, MessageRole::Assistant);
    assert_eq!(chat.messages[2].content, "Hi there!");
}

#[test]
fn test_chat_max_messages() {
    let mut chat = ChatView::new();

    // Add more than max messages (1000)
    for i in 0..1005 {
        chat.add_message(MessageRole::User, format!("Message {}", i));
    }

    // Should only keep last 1000
    assert_eq!(chat.messages.len(), 1000);
    assert_eq!(chat.messages.front().unwrap().content, "Message 5");
    assert_eq!(chat.messages.back().unwrap().content, "Message 1004");
}

#[test]
fn test_streaming_message() {
    let mut chat = ChatView::new();

    // ChatView starts with 1 welcome message
    assert_eq!(chat.messages.len(), 1);

    // Start streaming
    chat.start_streaming_message(MessageRole::Assistant);
    assert_eq!(chat.messages.len(), 2);
    assert!(chat.messages.back().unwrap().is_streaming);
    assert_eq!(chat.messages.back().unwrap().content, "");

    // Append content
    chat.append_to_streaming("Hello");
    assert_eq!(chat.messages.back().unwrap().content, "Hello");
    assert!(chat.messages.back().unwrap().is_streaming);

    chat.append_to_streaming(" World!");
    assert_eq!(chat.messages.back().unwrap().content, "Hello World!");

    // Finish streaming
    chat.finish_streaming();
    assert!(!chat.messages.back().unwrap().is_streaming);
}

#[test]
fn test_scroll_offset() {
    let mut chat = ChatView::new();

    // Add messages
    for i in 0..20 {
        chat.add_message(MessageRole::User, format!("Message {}", i));
    }

    // Initial scroll is at bottom
    assert_eq!(chat.scroll_offset, 0);

    // Can scroll up
    chat.scroll_offset = 10;
    assert_eq!(chat.scroll_offset, 10);

    // Scroll to bottom resets offset
    chat.scroll_to_bottom();
    assert_eq!(chat.scroll_offset, 0);
}

#[test]
fn test_text_wrapping() {
    use crate::ui::chat::textwrap;

    let text = "This is a very long line that should be wrapped at some point";
    let wrapped = textwrap::wrap(text, 20);

    assert_eq!(wrapped.len(), 4);
    assert_eq!(wrapped[0], "This is a very long");
    assert_eq!(wrapped[1], "line that should be");
    assert_eq!(wrapped[2], "wrapped at some");
    assert_eq!(wrapped[3], "point");

    // Test empty string
    let wrapped = textwrap::wrap("", 10);
    assert_eq!(wrapped.len(), 1);
    assert_eq!(wrapped[0], "");

    // Test single word longer than width
    let wrapped = textwrap::wrap("superlongword", 5);
    assert_eq!(wrapped.len(), 1);
    assert_eq!(wrapped[0], "superlongword");
}
