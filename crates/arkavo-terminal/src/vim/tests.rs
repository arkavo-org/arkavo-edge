use super::*;
use crossterm::event::KeyCode;

#[test]
fn test_vim_mode_transitions() {
    let mut state = VimState::new();

    assert_eq!(state.mode, VimMode::Normal);

    state.set_mode(VimMode::Insert);
    assert_eq!(state.mode, VimMode::Insert);

    state.set_mode(VimMode::Visual);
    assert_eq!(state.mode, VimMode::Visual);

    state.set_mode(VimMode::Normal);
    assert_eq!(state.mode, VimMode::Normal);
}

#[test]
fn test_vim_keybindings() {
    let bindings = VimKeyBinding::normal_mode_bindings();

    let insert_binding = bindings
        .iter()
        .find(|b| b.key.code == KeyCode::Char('i'))
        .expect("Should have 'i' binding");

    assert_eq!(insert_binding.command, VimCommand::EnterInsertMode);
    assert_eq!(insert_binding.mode, VimMode::Normal);
}

#[test]
fn test_motion_helpers() {
    assert!(motion::is_word_char('a'));
    assert!(motion::is_word_char('Z'));
    assert!(motion::is_word_char('0'));
    assert!(motion::is_word_char('_'));
    assert!(!motion::is_word_char(' '));
    assert!(!motion::is_word_char('.'));

    let text = "hello world test";
    assert_eq!(motion::find_word_boundary_forward(text, 0), 6);
    assert_eq!(motion::find_word_boundary_forward(text, 6), 12);
    assert_eq!(motion::find_word_boundary_backward(text, 12), 6);
    assert_eq!(motion::find_word_boundary_backward(text, 6), 0);
}
