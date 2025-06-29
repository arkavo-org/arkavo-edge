use super::input::InputField;
use crate::vim::{VimCommand, VimMode, VimState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct VimInputField {
    pub input: InputField,
    pub vim_state: VimState,
    pub vim_enabled: bool,
}

impl VimInputField {
    pub fn new(prompt: String) -> Self {
        Self {
            input: InputField::new(prompt),
            vim_state: VimState::new(),
            vim_enabled: true,
        }
    }

    pub fn with_placeholder(mut self, placeholder: String) -> Self {
        self.input = self.input.with_placeholder(placeholder);
        self
    }

    pub fn handle_key_event(&mut self, key: KeyEvent) -> Option<VimCommand> {
        if !self.vim_enabled {
            return self.handle_standard_key(key);
        }

        match self.vim_state.mode {
            VimMode::Normal => self.handle_normal_mode(key),
            VimMode::Insert => self.handle_insert_mode(key),
            VimMode::Visual => self.handle_visual_mode(key),
            VimMode::Command => self.handle_command_mode(key),
            _ => None,
        }
    }

    fn handle_standard_key(&mut self, key: KeyEvent) -> Option<VimCommand> {
        match key.code {
            KeyCode::Char(c) => {
                self.input.insert_char(c);
            }
            KeyCode::Backspace => {
                self.input.delete_char();
            }
            KeyCode::Delete => {
                self.input.delete_char_forward();
            }
            KeyCode::Left => {
                self.input.move_cursor_left();
            }
            KeyCode::Right => {
                self.input.move_cursor_right();
            }
            KeyCode::Home => {
                self.input.move_cursor_to_start();
            }
            KeyCode::End => {
                self.input.move_cursor_to_end();
            }
            _ => {}
        }
        None
    }

    fn handle_normal_mode(&mut self, key: KeyEvent) -> Option<VimCommand> {
        match (key.code, key.modifiers) {
            (KeyCode::Char('i'), KeyModifiers::NONE) => {
                self.vim_state.set_mode(VimMode::Insert);
                Some(VimCommand::EnterInsertMode)
            }
            (KeyCode::Char('a'), KeyModifiers::NONE) => {
                self.input.move_cursor_right();
                self.vim_state.set_mode(VimMode::Insert);
                Some(VimCommand::EnterInsertModeAfter)
            }
            (KeyCode::Char('A'), KeyModifiers::SHIFT) => {
                self.input.move_cursor_to_end();
                self.vim_state.set_mode(VimMode::Insert);
                Some(VimCommand::EnterInsertModeAfter)
            }
            (KeyCode::Char('I'), KeyModifiers::SHIFT) => {
                self.input.move_cursor_to_start();
                self.vim_state.set_mode(VimMode::Insert);
                Some(VimCommand::EnterInsertMode)
            }
            (KeyCode::Char('h'), KeyModifiers::NONE) => {
                self.input.move_cursor_left();
                Some(VimCommand::MoveLeft)
            }
            (KeyCode::Char('l'), KeyModifiers::NONE) => {
                self.input.move_cursor_right();
                Some(VimCommand::MoveRight)
            }
            (KeyCode::Char('0'), KeyModifiers::NONE) => {
                self.input.move_cursor_to_start();
                Some(VimCommand::MoveLineStart)
            }
            (KeyCode::Char('$'), KeyModifiers::NONE) => {
                self.input.move_cursor_to_end();
                Some(VimCommand::MoveLineEnd)
            }
            (KeyCode::Char('x'), KeyModifiers::NONE) => {
                self.input.delete_char_forward();
                Some(VimCommand::DeleteChar)
            }
            (KeyCode::Char('X'), KeyModifiers::SHIFT) => {
                self.input.delete_char();
                Some(VimCommand::DeleteChar)
            }
            (KeyCode::Char(':'), KeyModifiers::NONE) => {
                self.vim_state.set_mode(VimMode::Command);
                Some(VimCommand::EnterCommandMode)
            }
            (KeyCode::Char('v'), KeyModifiers::NONE) => {
                self.vim_state.set_mode(VimMode::Visual);
                self.vim_state.visual_start = Some((self.input.cursor_position, 0));
                Some(VimCommand::EnterVisualMode)
            }
            _ => None,
        }
    }

    fn handle_insert_mode(&mut self, key: KeyEvent) -> Option<VimCommand> {
        match key.code {
            KeyCode::Esc => {
                self.vim_state.set_mode(VimMode::Normal);
                self.input.move_cursor_left();
                Some(VimCommand::EnterNormalMode)
            }
            _ => self.handle_standard_key(key),
        }
    }

    fn handle_visual_mode(&mut self, key: KeyEvent) -> Option<VimCommand> {
        match key.code {
            KeyCode::Esc => {
                self.vim_state.set_mode(VimMode::Normal);
                Some(VimCommand::EnterNormalMode)
            }
            KeyCode::Char('h') => {
                self.input.move_cursor_left();
                Some(VimCommand::MoveLeft)
            }
            KeyCode::Char('l') => {
                self.input.move_cursor_right();
                Some(VimCommand::MoveRight)
            }
            _ => None,
        }
    }

    fn handle_command_mode(&mut self, key: KeyEvent) -> Option<VimCommand> {
        match key.code {
            KeyCode::Esc => {
                self.vim_state.set_mode(VimMode::Normal);
                self.vim_state.command_buffer.clear();
                Some(VimCommand::EnterNormalMode)
            }
            KeyCode::Enter => {
                let command = self.vim_state.command_buffer.clone();
                self.vim_state.set_mode(VimMode::Normal);
                self.execute_command(&command);
                Some(VimCommand::ExecuteCommand)
            }
            KeyCode::Char(c) => {
                self.vim_state.command_buffer.push(c);
                None
            }
            KeyCode::Backspace => {
                self.vim_state.command_buffer.pop();
                None
            }
            _ => None,
        }
    }

    fn execute_command(&self, command: &str) {
        match command {
            "q" | "quit" => {}
            "w" | "write" => {}
            "wq" => {}
            _ => {}
        }
    }

    pub fn get_mode_display(&self) -> String {
        if !self.vim_enabled {
            return String::new();
        }
        self.vim_state.mode.to_string()
    }

    pub fn get_command_line(&self) -> Option<String> {
        if self.vim_state.mode == VimMode::Command {
            Some(format!(":{}", self.vim_state.command_buffer))
        } else {
            None
        }
    }
}
