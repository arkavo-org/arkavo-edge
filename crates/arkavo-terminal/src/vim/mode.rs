use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Insert,
    Visual,
    VisualLine,
    VisualBlock,
    Command,
    Replace,
}

impl fmt::Display for VimMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VimMode::Normal => write!(f, "NORMAL"),
            VimMode::Insert => write!(f, "INSERT"),
            VimMode::Visual => write!(f, "VISUAL"),
            VimMode::VisualLine => write!(f, "V-LINE"),
            VimMode::VisualBlock => write!(f, "V-BLOCK"),
            VimMode::Command => write!(f, "COMMAND"),
            VimMode::Replace => write!(f, "REPLACE"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VimState {
    pub mode: VimMode,
    pub command_buffer: String,
    pub count: Option<usize>,
    pub register: Option<char>,
    pub last_command: Option<String>,
    pub visual_start: Option<(usize, usize)>,
    pub search_pattern: Option<String>,
    pub search_forward: bool,
}

impl Default for VimState {
    fn default() -> Self {
        Self {
            mode: VimMode::Normal,
            command_buffer: String::new(),
            count: None,
            register: None,
            last_command: None,
            visual_start: None,
            search_pattern: None,
            search_forward: true,
        }
    }
}

impl VimState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_mode(&mut self, mode: VimMode) {
        self.mode = mode;
        self.count = None;
        self.register = None;

        if mode != VimMode::Visual && mode != VimMode::VisualLine && mode != VimMode::VisualBlock {
            self.visual_start = None;
        }

        if mode != VimMode::Command {
            self.command_buffer.clear();
        }
    }

    pub fn is_visual_mode(&self) -> bool {
        matches!(
            self.mode,
            VimMode::Visual | VimMode::VisualLine | VimMode::VisualBlock
        )
    }

    pub fn get_count(&self) -> usize {
        self.count.unwrap_or(1)
    }

    pub fn append_count(&mut self, digit: char) {
        if let Some(d) = digit.to_digit(10) {
            let current = self.count.unwrap_or(0);
            self.count = Some(current * 10 + d as usize);
        }
    }

    pub fn reset_count(&mut self) {
        self.count = None;
    }
}
