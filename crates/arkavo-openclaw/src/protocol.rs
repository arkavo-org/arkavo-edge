use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Top-level OpenClaw WebSocket frame envelope.
///
/// All messages are JSON text frames tagged by `"type"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OpenClawFrame {
    #[serde(rename = "connect")]
    Connect(ConnectFrame),
    #[serde(rename = "connect.challenge")]
    Challenge(ChallengeFrame),
    #[serde(rename = "hello-ok")]
    HelloOk(HelloOkFrame),
    #[serde(rename = "req")]
    Request(RequestFrame),
    #[serde(rename = "event")]
    Event(EventFrame),
    #[serde(rename = "res")]
    Response(ResponseFrame),
    #[serde(rename = "error")]
    Error(ErrorFrame),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectFrame {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<ConnectAuth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Vec<String>>,
    #[serde(rename = "minProtocol", skip_serializing_if = "Option::is_none")]
    pub min_protocol: Option<String>,
    #[serde(rename = "maxProtocol", skip_serializing_if = "Option::is_none")]
    pub max_protocol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectAuth {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeFrame {
    pub nonce: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloOkFrame {
    #[serde(rename = "deviceToken")]
    pub device_token: String,
    #[serde(rename = "serverId", skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestFrame {
    pub id: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFrame {
    pub topic: String,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFrame {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<OpenClawError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenClawError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorFrame {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_frame_round_trip() {
        let frame = OpenClawFrame::Connect(ConnectFrame {
            auth: Some(ConnectAuth {
                token: Some("abc123".to_string()),
                password: None,
            }),
            role: Some("operator".to_string()),
            scope: Some(vec!["chat".to_string(), "tasks".to_string()]),
            min_protocol: Some("1".to_string()),
            max_protocol: Some("2".to_string()),
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""type":"connect"#));
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Connect(c) => {
                assert_eq!(c.auth.unwrap().token.unwrap(), "abc123");
                assert_eq!(c.role.unwrap(), "operator");
            }
            _ => panic!("expected Connect"),
        }
    }

    #[test]
    fn challenge_frame_round_trip() {
        let frame = OpenClawFrame::Challenge(ChallengeFrame {
            nonce: "deadbeef".to_string(),
            timestamp: 1_700_000_000,
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""type":"connect.challenge"#));
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Challenge(c) => {
                assert_eq!(c.nonce, "deadbeef");
                assert_eq!(c.timestamp, 1_700_000_000);
            }
            _ => panic!("expected Challenge"),
        }
    }

    #[test]
    fn hello_ok_frame_round_trip() {
        let frame = OpenClawFrame::HelloOk(HelloOkFrame {
            device_token: "tok_xyz".to_string(),
            server_id: Some("srv-1".to_string()),
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""deviceToken"#));
        assert!(json.contains(r#""serverId"#));
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::HelloOk(h) => {
                assert_eq!(h.device_token, "tok_xyz");
                assert_eq!(h.server_id.unwrap(), "srv-1");
            }
            _ => panic!("expected HelloOk"),
        }
    }

    #[test]
    fn request_frame_round_trip() {
        let frame = OpenClawFrame::Request(RequestFrame {
            id: "req-1".to_string(),
            method: "chat".to_string(),
            params: Some(serde_json::json!({"content": "hello"})),
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""type":"req"#));
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Request(r) => {
                assert_eq!(r.id, "req-1");
                assert_eq!(r.method, "chat");
            }
            _ => panic!("expected Request"),
        }
    }

    #[test]
    fn event_frame_round_trip() {
        let frame = OpenClawFrame::Event(EventFrame {
            topic: "chat.delta".to_string(),
            data: serde_json::json!({"text": "hi"}),
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""type":"event"#));
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Event(e) => {
                assert_eq!(e.topic, "chat.delta");
            }
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn response_frame_success_round_trip() {
        let frame = OpenClawFrame::Response(ResponseFrame {
            id: "req-1".to_string(),
            result: Some(serde_json::json!({"status": "ok"})),
            error: None,
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""type":"res"#));
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Response(r) => {
                assert_eq!(r.id, "req-1");
                assert!(r.result.is_some());
                assert!(r.error.is_none());
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn response_frame_error_round_trip() {
        let frame = OpenClawFrame::Response(ResponseFrame {
            id: "req-2".to_string(),
            result: None,
            error: Some(OpenClawError {
                code: -1,
                message: "not found".to_string(),
                data: None,
            }),
        });
        let json = serde_json::to_string(&frame).unwrap();
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Response(r) => {
                assert!(r.error.is_some());
                assert_eq!(r.error.unwrap().code, -1);
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn error_frame_round_trip() {
        let frame = OpenClawFrame::Error(ErrorFrame {
            code: 4001,
            message: "auth failed".to_string(),
            data: None,
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""type":"error"#));
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Error(e) => {
                assert_eq!(e.code, 4001);
                assert_eq!(e.message, "auth failed");
            }
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn connect_frame_minimal() {
        let json = r#"{"type":"connect"}"#;
        let parsed: OpenClawFrame = serde_json::from_str(json).unwrap();
        match parsed {
            OpenClawFrame::Connect(c) => {
                assert!(c.auth.is_none());
                assert!(c.role.is_none());
            }
            _ => panic!("expected Connect"),
        }
    }

    #[test]
    fn hello_ok_without_server_id() {
        let json = r#"{"type":"hello-ok","deviceToken":"tok"}"#;
        let parsed: OpenClawFrame = serde_json::from_str(json).unwrap();
        match parsed {
            OpenClawFrame::HelloOk(h) => {
                assert_eq!(h.device_token, "tok");
                assert!(h.server_id.is_none());
            }
            _ => panic!("expected HelloOk"),
        }
    }
}
