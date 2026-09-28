#![allow(clippy::disallowed_methods)]
#![allow(clippy::future_not_send)]
#![allow(dead_code)]
#![allow(clippy::format_push_string)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unnecessary_debug_formatting)]
#![allow(clippy::lines_filter_map_ok)]
#![allow(clippy::manual_strip)]
#![allow(clippy::needless_continue)]
#![allow(unused_imports)]
#![allow(clippy::zombie_processes)]
#![allow(clippy::unreadable_literal)]
#![allow(clippy::ignore_without_reason)]
#![allow(clippy::unnecessary_unwrap)]
#![allow(unreachable_pub)]

use std::net::TcpListener;
use tempfile::TempDir;

pub struct TestEnvironment {
    pub temp_dir: TempDir,
    pub ui_port: u16,
    pub agent_ports: Vec<u16>,
}

impl TestEnvironment {
    pub(crate) fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;
        let ui_port = get_free_port()?;

        // Create .arkavo directory for task database
        let arkavo_dir = temp_dir.path().join(".arkavo");
        std::fs::create_dir(&arkavo_dir)?;

        Ok(Self {
            temp_dir,
            ui_port,
            agent_ports: Vec::new(),
        })
    }

    pub(crate) fn with_agents(mut self, count: usize) -> Result<Self, Box<dyn std::error::Error>> {
        for _ in 0..count {
            self.agent_ports.push(get_free_port()?);
        }
        Ok(self)
    }
}

pub fn get_free_port() -> Result<u16, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

pub fn create_test_message(content: &str) -> serde_json::Value {
    serde_json::json!({
        "message": content,
        "context": {
            "test": true,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }
    })
}
