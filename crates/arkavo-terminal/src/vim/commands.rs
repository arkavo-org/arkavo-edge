#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimCommand {
    EnterNormalMode,
    EnterInsertMode,
    EnterInsertModeAfter,
    EnterVisualMode,
    EnterVisualLineMode,
    EnterVisualBlockMode,
    EnterCommandMode,
    EnterReplaceMode,

    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveWordForward,
    MoveWordBackward,
    MoveWordEnd,
    MoveLineStart,
    MoveLineEnd,
    GoToTop,
    GoToBottom,
    GoToLine,

    DeleteChar,
    DeleteMotion,
    DeleteLine,
    DeleteSelection,

    YankMotion,
    YankLine,
    YankSelection,

    PasteAfter,
    PasteBefore,

    OpenLineBelow,
    OpenLineAbove,

    Undo,
    Redo,

    SearchForward,
    SearchBackward,
    SearchNext,
    SearchPrevious,

    ExecuteCommand,
    Prefix,

    None,
}

impl VimCommand {
    pub fn is_motion(&self) -> bool {
        matches!(
            self,
            VimCommand::MoveLeft
                | VimCommand::MoveRight
                | VimCommand::MoveUp
                | VimCommand::MoveDown
                | VimCommand::MoveWordForward
                | VimCommand::MoveWordBackward
                | VimCommand::MoveWordEnd
                | VimCommand::MoveLineStart
                | VimCommand::MoveLineEnd
                | VimCommand::GoToTop
                | VimCommand::GoToBottom
                | VimCommand::GoToLine
        )
    }

    pub fn requires_motion(&self) -> bool {
        matches!(self, VimCommand::DeleteMotion | VimCommand::YankMotion)
    }
}
