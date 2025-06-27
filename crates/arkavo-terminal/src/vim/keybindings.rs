use super::commands::VimCommand;
use super::mode::VimMode;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone)]
pub struct VimKeyBinding {
    pub key: KeyEvent,
    pub mode: VimMode,
    pub command: VimCommand,
}

impl VimKeyBinding {
    pub fn new(key: KeyEvent, mode: VimMode, command: VimCommand) -> Self {
        Self { key, mode, command }
    }

    pub fn normal_mode_bindings() -> Vec<Self> {
        vec![
            Self::new(
                KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::EnterInsertMode,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::EnterInsertModeAfter,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::OpenLineBelow,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT),
                VimMode::Normal,
                VimCommand::OpenLineAbove,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::EnterVisualMode,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT),
                VimMode::Normal,
                VimCommand::EnterVisualLineMode,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
                VimMode::Normal,
                VimCommand::EnterVisualBlockMode,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::EnterCommandMode,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::MoveLeft,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::MoveDown,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::MoveUp,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::MoveRight,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::MoveWordForward,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::MoveWordBackward,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::MoveWordEnd,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::MoveLineStart,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::MoveLineEnd,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::Prefix,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT),
                VimMode::Normal,
                VimCommand::GoToBottom,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::DeleteChar,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::DeleteMotion,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::YankMotion,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::PasteAfter,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT),
                VimMode::Normal,
                VimCommand::PasteBefore,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::Undo,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
                VimMode::Normal,
                VimCommand::Redo,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::SearchForward,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::SearchBackward,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
                VimMode::Normal,
                VimCommand::SearchNext,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT),
                VimMode::Normal,
                VimCommand::SearchPrevious,
            ),
        ]
    }

    pub fn insert_mode_bindings() -> Vec<Self> {
        vec![
            Self::new(
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                VimMode::Insert,
                VimCommand::EnterNormalMode,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                VimMode::Insert,
                VimCommand::EnterNormalMode,
            ),
        ]
    }

    pub fn visual_mode_bindings() -> Vec<Self> {
        vec![
            Self::new(
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                VimMode::Visual,
                VimCommand::EnterNormalMode,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
                VimMode::Visual,
                VimCommand::MoveLeft,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                VimMode::Visual,
                VimCommand::MoveDown,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
                VimMode::Visual,
                VimCommand::MoveUp,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
                VimMode::Visual,
                VimCommand::MoveRight,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                VimMode::Visual,
                VimCommand::DeleteSelection,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                VimMode::Visual,
                VimCommand::YankSelection,
            ),
        ]
    }

    pub fn command_mode_bindings() -> Vec<Self> {
        vec![
            Self::new(
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                VimMode::Command,
                VimCommand::EnterNormalMode,
            ),
            Self::new(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                VimMode::Command,
                VimCommand::ExecuteCommand,
            ),
        ]
    }

    pub fn all_bindings() -> Vec<Self> {
        let mut bindings = Vec::new();
        bindings.extend(Self::normal_mode_bindings());
        bindings.extend(Self::insert_mode_bindings());
        bindings.extend(Self::visual_mode_bindings());
        bindings.extend(Self::command_mode_bindings());
        bindings
    }
}
