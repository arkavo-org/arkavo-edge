//! MCP server declarations for kits (spec §4.x): how a role's `mcp_tools`
//! grant is actually launched/connected at play time.

use serde::{Deserialize, Serialize};

/// Transport for an MCP server connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Stdio,
    Http,
    Sse,
}

/// A named MCP server a kit can launch/connect. Roles reference it by `name`
/// from their `mcp_tools[].server`. `command`/`args` may contain `${VAR}`
/// placeholders expanded at play time (not parse time), keeping the kit
/// portable and its `kit.id` stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerDef {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub transport: Transport,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_minimal_server() {
        let yaml = r#"
name: game-rl
command: ${GAME_RL_SERVER}
"#;
        let def: McpServerDef = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.name, "game-rl");
        assert_eq!(def.command, "${GAME_RL_SERVER}");
        assert!(def.args.is_empty());
        assert_eq!(def.transport, Transport::Stdio);
    }

    #[test]
    fn round_trips_with_args_and_transport() {
        let def = McpServerDef {
            name: "game-rl".into(),
            command: "game-rl-server".into(),
            args: vec!["--http".into()],
            transport: Transport::Http,
        };
        let yaml = serde_yaml::to_string(&def).unwrap();
        let back: McpServerDef = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(def, back);
    }
}
