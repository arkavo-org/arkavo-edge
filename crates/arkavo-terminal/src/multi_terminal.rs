use anyhow::{Result, anyhow};
use std::env;
use std::process::Command;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct TerminalSpawnConfig {
    pub session_id: Uuid,
    pub task_id: String,
    pub task_type: TaskType,
    pub title: String,
}

impl TerminalSpawnConfig {
    pub fn validate(&self) -> Result<()> {
        // Validate task_id doesn't contain shell-unsafe characters
        if self
            .task_id
            .contains(|c: char| c.is_whitespace() || "$`\"'\\|<>&;(){}".contains(c))
        {
            return Err(anyhow!("Invalid task_id: contains unsafe characters"));
        }

        // Validate title doesn't contain unsafe characters
        if self.title.contains(['\n', '\r', '\0']) {
            return Err(anyhow!("Invalid title: contains unsafe characters"));
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum TaskType {
    ChainOfThought,
    CodeEditing,
    TestOutput,
    DiffPreview,
}

impl TaskType {
    pub fn as_str(&self) -> &str {
        match self {
            TaskType::ChainOfThought => "cot",
            TaskType::CodeEditing => "code",
            TaskType::TestOutput => "test",
            TaskType::DiffPreview => "diff",
        }
    }
}

pub struct MultiTerminalManager {
    executable_path: String,
}

impl MultiTerminalManager {
    pub fn new() -> Result<Self> {
        let executable_path = env::current_exe()?
            .to_str()
            .ok_or_else(|| anyhow!("Invalid executable path"))?
            .to_string();

        Ok(Self { executable_path })
    }

    pub fn spawn_terminal(&self, config: TerminalSpawnConfig) -> Result<()> {
        // Validate config to prevent injection attacks
        config.validate()?;

        let terminal_app = self.detect_terminal()?;

        match terminal_app {
            TerminalApp::MacTerminal => self.spawn_mac_terminal(&config),
            TerminalApp::ITerm2 => self.spawn_iterm2(&config),
            TerminalApp::WezTerm => self.spawn_wezterm(&config),
            TerminalApp::Tmux => self.spawn_tmux_pane(&config),
            TerminalApp::Zellij => self.spawn_zellij_pane(&config),
            TerminalApp::Generic => self.spawn_generic_terminal(&config),
        }
    }

    fn detect_terminal(&self) -> Result<TerminalApp> {
        // Check for terminal multiplexers first
        if env::var("TMUX").is_ok() {
            return Ok(TerminalApp::Tmux);
        }

        if env::var("ZELLIJ").is_ok() {
            return Ok(TerminalApp::Zellij);
        }

        // Check for specific terminals on macOS
        #[cfg(target_os = "macos")]
        {
            if env::var("TERM_PROGRAM").ok().as_deref() == Some("WezTerm") {
                return Ok(TerminalApp::WezTerm);
            }

            if env::var("TERM_PROGRAM").ok().as_deref() == Some("iTerm.app") {
                return Ok(TerminalApp::ITerm2);
            }

            if env::var("TERM_PROGRAM").ok().as_deref() == Some("Apple_Terminal") {
                return Ok(TerminalApp::MacTerminal);
            }
        }

        // Default to generic terminal
        Ok(TerminalApp::Generic)
    }

    fn spawn_mac_terminal(&self, config: &TerminalSpawnConfig) -> Result<()> {
        let command = format!(
            "{} view --task {} --session {}",
            self.executable_path, config.task_id, config.session_id
        );

        Command::new("osascript")
            .arg("-e")
            .arg(format!("tell app \"Terminal\" to do script \"{command}\""))
            .spawn()?;

        Ok(())
    }

    fn spawn_iterm2(&self, config: &TerminalSpawnConfig) -> Result<()> {
        let command = format!(
            "{} view --task {} --session {}",
            self.executable_path, config.task_id, config.session_id
        );

        Command::new("osascript")
            .arg("-e")
            .arg(format!(
                "tell application \"iTerm2\"
                    tell current window
                        create tab with default profile
                        tell current session
                            write text \"{command}\"
                        end tell
                    end tell
                end tell"
            ))
            .spawn()?;

        Ok(())
    }

    fn spawn_wezterm(&self, config: &TerminalSpawnConfig) -> Result<()> {
        let command = format!(
            "{} view --task {} --session {}",
            self.executable_path, config.task_id, config.session_id
        );

        Command::new("wezterm")
            .args(["cli", "spawn", "--", "sh", "-c", &command])
            .spawn()?;

        Ok(())
    }

    fn spawn_tmux_pane(&self, config: &TerminalSpawnConfig) -> Result<()> {
        Command::new("tmux")
            .args([
                "split-window",
                "-h",
                "-p",
                "50",
                &self.executable_path,
                "view",
                "--task",
                &config.task_id,
                "--session",
                &config.session_id.to_string(),
            ])
            .spawn()?;

        Ok(())
    }

    fn spawn_zellij_pane(&self, config: &TerminalSpawnConfig) -> Result<()> {
        let _command = format!(
            "{} view --task {} --session {}",
            self.executable_path, config.task_id, config.session_id
        );

        Command::new("zellij")
            .args([
                "action",
                "new-pane",
                "--",
                &self.executable_path,
                "view",
                "--task",
                &config.task_id,
                "--session",
                &config.session_id.to_string(),
            ])
            .spawn()?;

        Ok(())
    }

    #[cfg(unix)]
    fn spawn_generic_terminal(&self, config: &TerminalSpawnConfig) -> Result<()> {
        // Try common terminal emulators (WezTerm first)
        let terminals = [
            "wezterm",
            "x-terminal-emulator",
            "gnome-terminal",
            "konsole",
            "xterm",
        ];

        for terminal in &terminals {
            let result = if terminal == &"wezterm" {
                // WezTerm uses different syntax
                Command::new(terminal)
                    .arg("start")
                    .arg("--")
                    .arg(&self.executable_path)
                    .arg("view")
                    .arg("--task")
                    .arg(&config.task_id)
                    .arg("--session")
                    .arg(config.session_id.to_string())
                    .spawn()
            } else {
                // Other terminals use -e flag
                Command::new(terminal)
                    .arg("-e")
                    .arg(&self.executable_path)
                    .arg("view")
                    .arg("--task")
                    .arg(&config.task_id)
                    .arg("--session")
                    .arg(config.session_id.to_string())
                    .spawn()
            };

            if result.is_ok() {
                return Ok(());
            }
        }

        Err(anyhow!("Could not find a suitable terminal emulator"))
    }

    #[cfg(not(unix))]
    fn spawn_generic_terminal(&self, _config: &TerminalSpawnConfig) -> Result<()> {
        Err(anyhow!(
            "Multi-terminal spawning not yet implemented for Windows"
        ))
    }
}

#[derive(Debug)]
#[allow(dead_code)] // Some variants are platform-specific
enum TerminalApp {
    MacTerminal,
    ITerm2,
    WezTerm,
    Tmux,
    Zellij,
    Generic,
}

pub fn check_multiplexer_support() -> MultiplexerSupport {
    MultiplexerSupport {
        tmux: env::var("TMUX").is_ok(),
        zellij: env::var("ZELLIJ").is_ok(),
        screen: env::var("STY").is_ok(),
    }
}

#[derive(Debug)]
pub struct MultiplexerSupport {
    pub tmux: bool,
    pub zellij: bool,
    pub screen: bool,
}

impl MultiplexerSupport {
    pub fn has_any(&self) -> bool {
        self.tmux || self.zellij || self.screen
    }

    pub fn preferred(&self) -> Option<&'static str> {
        if self.tmux {
            Some("tmux")
        } else if self.zellij {
            Some("zellij")
        } else if self.screen {
            Some("screen")
        } else {
            None
        }
    }
}
