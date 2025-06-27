#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    CharLeft(usize),
    CharRight(usize),
    LineUp(usize),
    LineDown(usize),
    WordForward(usize),
    WordBackward(usize),
    WordEnd(usize),
    LineStart,
    LineEnd,
    FirstLine,
    LastLine,
    Line(usize),
}

impl Motion {
    pub fn from_count(base: Motion, count: usize) -> Self {
        match base {
            Motion::CharLeft(_) => Motion::CharLeft(count),
            Motion::CharRight(_) => Motion::CharRight(count),
            Motion::LineUp(_) => Motion::LineUp(count),
            Motion::LineDown(_) => Motion::LineDown(count),
            Motion::WordForward(_) => Motion::WordForward(count),
            Motion::WordBackward(_) => Motion::WordBackward(count),
            Motion::WordEnd(_) => Motion::WordEnd(count),
            _ => base,
        }
    }
}

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn find_word_boundary_forward(text: &str, pos: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut i = pos;

    if i >= chars.len() {
        return chars.len();
    }

    if is_word_char(chars[i]) {
        while i < chars.len() && is_word_char(chars[i]) {
            i += 1;
        }
    } else if !chars[i].is_whitespace() {
        while i < chars.len() && !is_word_char(chars[i]) && !chars[i].is_whitespace() {
            i += 1;
        }
    }

    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }

    i
}

pub fn find_word_boundary_backward(text: &str, pos: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();

    if pos == 0 {
        return 0;
    }

    let mut i = pos.saturating_sub(1);

    while i > 0 && chars[i].is_whitespace() {
        i = i.saturating_sub(1);
    }

    if i > 0 {
        if is_word_char(chars[i]) {
            while i > 0 && is_word_char(chars[i.saturating_sub(1)]) {
                i = i.saturating_sub(1);
            }
        } else {
            while i > 0
                && !is_word_char(chars[i.saturating_sub(1)])
                && !chars[i.saturating_sub(1)].is_whitespace()
            {
                i = i.saturating_sub(1);
            }
        }
    }

    i
}

pub fn find_word_end(text: &str, pos: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut i = pos;

    if i >= chars.len().saturating_sub(1) {
        return chars.len().saturating_sub(1);
    }

    i += 1;

    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }

    if i < chars.len() {
        if is_word_char(chars[i]) {
            while i < chars.len().saturating_sub(1) && is_word_char(chars[i + 1]) {
                i += 1;
            }
        } else {
            while i < chars.len().saturating_sub(1)
                && !is_word_char(chars[i + 1])
                && !chars[i + 1].is_whitespace()
            {
                i += 1;
            }
        }
    }

    i
}
