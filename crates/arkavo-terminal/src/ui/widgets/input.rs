use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};
use unicode_width::UnicodeWidthStr;

pub struct InputField {
    pub prompt: String,
    pub value: String,
    pub cursor_position: usize,
    pub cursor_visible: bool,
    pub placeholder: String,
}

impl InputField {
    pub fn new(prompt: String) -> Self {
        Self {
            prompt,
            value: String::new(),
            cursor_position: 0,
            cursor_visible: true,
            placeholder: String::new(),
        }
    }

    pub fn with_placeholder(mut self, placeholder: String) -> Self {
        self.placeholder = placeholder;
        self
    }

    pub fn insert_char(&mut self, ch: char) {
        let index = self.byte_index();
        self.value.insert(index, ch);
        self.move_cursor_right();
    }

    pub fn delete_char(&mut self) {
        if self.cursor_position > 0 {
            let index = self.byte_index();
            let ch = self.value.chars().nth(self.cursor_position - 1).unwrap();
            self.value.remove(index - ch.len_utf8());
            self.move_cursor_left();
        }
    }

    pub fn delete_char_forward(&mut self) {
        if self.cursor_position < self.value.chars().count() {
            let index = self.byte_index();
            let ch = self.value.chars().nth(self.cursor_position).unwrap();
            self.value.drain(index..index + ch.len_utf8());
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor_position < self.value.chars().count() {
            self.cursor_position += 1;
        }
    }

    pub fn move_cursor_to_start(&mut self) {
        self.cursor_position = 0;
    }

    pub fn move_cursor_to_end(&mut self) {
        self.cursor_position = self.value.chars().count();
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor_position = 0;
    }

    fn byte_index(&self) -> usize {
        self.value
            .chars()
            .take(self.cursor_position)
            .map(char::len_utf8)
            .sum()
    }

    pub fn render(&self, frame: &mut ratatui::Frame, area: Rect) {
        let mut spans = vec![
            Span::styled(&self.prompt, Style::default().fg(Color::Cyan)),
            Span::raw(" "),
        ];

        if self.value.is_empty() && !self.placeholder.is_empty() {
            spans.push(Span::styled(
                &self.placeholder,
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            let value_chars: Vec<char> = self.value.chars().collect();

            if self.cursor_position > 0 {
                let before_cursor: String = value_chars[..self.cursor_position].iter().collect();
                spans.push(Span::raw(before_cursor));
            }

            if self.cursor_visible {
                if self.cursor_position < value_chars.len() {
                    spans.push(Span::styled(
                        value_chars[self.cursor_position].to_string(),
                        Style::default().bg(Color::White).fg(Color::Black),
                    ));

                    if self.cursor_position + 1 < value_chars.len() {
                        let after_cursor: String =
                            value_chars[self.cursor_position + 1..].iter().collect();
                        spans.push(Span::raw(after_cursor));
                    }
                } else {
                    spans.push(Span::styled(" ", Style::default().bg(Color::White)));
                }
            } else if self.cursor_position < value_chars.len() {
                let after_cursor: String = value_chars[self.cursor_position..].iter().collect();
                spans.push(Span::raw(after_cursor));
            }
        }

        let paragraph = Paragraph::new(Line::from(spans));
        frame.render_widget(paragraph, area);
    }
}

impl Widget for InputField {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let block = Block::default().borders(Borders::NONE);

        let inner = block.inner(area);
        block.render(area, buf);

        let display_text = if self.value.is_empty() && !self.placeholder.is_empty() {
            self.placeholder.clone()
        } else {
            self.value.clone()
        };

        let width = display_text.width() as u16;
        let scroll = if width > inner.width {
            width.saturating_sub(inner.width)
        } else {
            0
        };

        for (i, ch) in display_text.chars().skip(scroll as usize).enumerate() {
            if inner.x + i as u16 >= inner.right() {
                break;
            }

            let style = if self.value.is_empty() && !self.placeholder.is_empty() {
                Style::default().fg(Color::DarkGray)
            } else if self.cursor_visible && i == self.cursor_position - scroll as usize {
                Style::default().bg(Color::White).fg(Color::Black)
            } else {
                Style::default()
            };

            buf[(inner.x + i as u16, inner.y)]
                .set_char(ch)
                .set_style(style);
        }
    }
}
