use crate::vim::{VimCommand, VimMode};
use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone)]
pub enum AppEvent {
    KeyPress(KeyEvent),
    MouseClick(u16, u16),
    Resize(u16, u16),
    Tick,
    VimCommand(VimCommand),
    VimModeChange(VimMode),
}

impl AppEvent {
    pub fn from_crossterm_event(event: CrosstermEvent) -> Self {
        match event {
            CrosstermEvent::Key(key) => AppEvent::KeyPress(key),
            CrosstermEvent::Mouse(mouse) => AppEvent::MouseClick(mouse.column, mouse.row),
            CrosstermEvent::Resize(width, height) => AppEvent::Resize(width, height),
            _ => AppEvent::Tick,
        }
    }
}

pub struct EventHandler {
    pub shortcuts: Vec<Shortcut>,
}

pub struct Shortcut {
    pub key: KeyCode,
    pub modifiers: KeyModifiers,
    pub description: &'static str,
}

impl EventHandler {
    pub fn new() -> Self {
        Self {
            shortcuts: vec![
                Shortcut {
                    key: KeyCode::Char('q'),
                    modifiers: KeyModifiers::NONE,
                    description: "Quit application",
                },
                Shortcut {
                    key: KeyCode::Tab,
                    modifiers: KeyModifiers::NONE,
                    description: "Switch view",
                },
                Shortcut {
                    key: KeyCode::Char('c'),
                    modifiers: KeyModifiers::CONTROL,
                    description: "Copy selection",
                },
                Shortcut {
                    key: KeyCode::Char('v'),
                    modifiers: KeyModifiers::CONTROL,
                    description: "Paste",
                },
                Shortcut {
                    key: KeyCode::Up,
                    modifiers: KeyModifiers::NONE,
                    description: "Scroll up",
                },
                Shortcut {
                    key: KeyCode::Down,
                    modifiers: KeyModifiers::NONE,
                    description: "Scroll down",
                },
            ],
        }
    }
}

impl Default for EventHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl EventHandler {
    pub fn get_help_text(&self) -> Vec<String> {
        self.shortcuts
            .iter()
            .map(|s| {
                let key = match s.key {
                    KeyCode::Char(c) => {
                        if s.modifiers.contains(KeyModifiers::CONTROL) {
                            format!("Ctrl+{c}")
                        } else {
                            c.to_string()
                        }
                    }
                    KeyCode::Tab => "Tab".to_string(),
                    KeyCode::Up => "↑".to_string(),
                    KeyCode::Down => "↓".to_string(),
                    _ => format!("{:?}", s.key),
                };
                format!("{}: {}", key, s.description)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[test]
    #[spec("TERM-002")]
    fn crossterm_events_map_to_app_events() {
        let key = CrosstermEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(matches!(
            AppEvent::from_crossterm_event(key),
            AppEvent::KeyPress(_)
        ));

        let resize = CrosstermEvent::Resize(120, 40);
        match AppEvent::from_crossterm_event(resize) {
            AppEvent::Resize(w, h) => {
                assert_eq!(w, 120);
                assert_eq!(h, 40);
            }
            other => panic!("expected Resize, got {other:?}"),
        }

        // Events the app does not handle collapse to a harmless Tick
        let focus = CrosstermEvent::FocusGained;
        assert!(matches!(
            AppEvent::from_crossterm_event(focus),
            AppEvent::Tick
        ));
    }

    #[test]
    #[spec("TERM-002")]
    fn event_handler_registers_shortcuts_with_help_text() {
        let handler = EventHandler::new();

        assert!(!handler.shortcuts.is_empty());
        assert!(
            handler
                .shortcuts
                .iter()
                .any(|s| s.key == KeyCode::Char('q') && s.description.contains("Quit"))
        );

        let help = handler.get_help_text();
        assert_eq!(help.len(), handler.shortcuts.len());
        assert!(help.iter().any(|line| line.contains("Ctrl+c")));
        assert!(help.iter().any(|line| line.starts_with("Tab:")));
    }
}
