use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Top-level OpenClaw WebSocket frame envelope.
///
/// OpenClaw uses three wire-level frame types: `req`, `res`, and `event`.
/// The handshake (challenge, connect, hello-ok) rides on these same types:
///   - Challenge arrives as `event` with `event: "connect.challenge"`
///   - Connect is sent as `req` with `method: "connect"`
///   - Hello-ok arrives as `res` with `payload.type: "hello-ok"`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OpenClawFrame {
    #[serde(rename = "req")]
    Request(RequestFrame),
    #[serde(rename = "res")]
    Response(ResponseFrame),
    #[serde(rename = "event")]
    Event(EventFrame),
}

// ── Request frame ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestFrame {
    pub id: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

// ── Response frame ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFrame {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<OpenClawError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenClawError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// ── Event frame ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFrame {
    pub event: String,
    pub payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(rename = "stateVersion", skip_serializing_if = "Option::is_none")]
    pub state_version: Option<Value>,
}

// ── Connect handshake structures ───────────────────────────────────────────

/// Challenge payload inside an event frame (`event: "connect.challenge"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengePayload {
    pub nonce: String,
    pub ts: u64,
}

/// Client info required in the connect request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    pub id: String,
    pub version: String,
    pub platform: String,
    pub mode: String,
    #[serde(rename = "displayName", skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(rename = "instanceId", skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
}

/// Auth credentials in the connect request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectAuth {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(rename = "deviceToken", skip_serializing_if = "Option::is_none")]
    pub device_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

/// Parameters for the `connect` request method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectParams {
    #[serde(rename = "minProtocol")]
    pub min_protocol: u32,
    #[serde(rename = "maxProtocol")]
    pub max_protocol: u32,
    pub client: ClientInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caps: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<ConnectAuth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<crate::device::SignedDevice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(rename = "userAgent", skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
}

/// Hello-ok payload inside a response frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloOkPayload {
    #[serde(rename = "type")]
    pub payload_type: String,
    pub protocol: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<HelloOkAuth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub version: String,
    #[serde(rename = "connId")]
    pub conn_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloOkAuth {
    #[serde(rename = "deviceToken")]
    pub device_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
}

/// Current protocol version supported by this implementation.
pub const PROTOCOL_VERSION: u32 = 3;

// ── Helper constructors ────────────────────────────────────────────────────

/// Build a challenge event frame (server -> client).
pub fn challenge_event(nonce: &str, ts: u64) -> OpenClawFrame {
    OpenClawFrame::Event(EventFrame {
        event: "connect.challenge".to_string(),
        payload: serde_json::json!({ "nonce": nonce, "ts": ts }),
        seq: None,
        state_version: None,
    })
}

/// Build a connect request frame (client -> server).
pub fn connect_request(
    id: &str,
    params: ConnectParams,
) -> Result<OpenClawFrame, serde_json::Error> {
    Ok(OpenClawFrame::Request(RequestFrame {
        id: id.to_string(),
        method: "connect".to_string(),
        params: Some(serde_json::to_value(params)?),
    }))
}

/// Build a hello-ok response frame (server -> client).
pub fn hello_ok_response(
    id: &str,
    protocol: u32,
    conn_id: &str,
) -> Result<OpenClawFrame, serde_json::Error> {
    let payload = HelloOkPayload {
        payload_type: "hello-ok".to_string(),
        protocol,
        server: Some(ServerInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            conn_id: conn_id.to_string(),
        }),
        auth: None,
        policy: Some(serde_json::json!({
            "tickIntervalMs": 30000
        })),
        features: None,
    };
    Ok(OpenClawFrame::Response(ResponseFrame {
        id: id.to_string(),
        ok: true,
        payload: Some(serde_json::to_value(payload)?),
        error: None,
    }))
}

/// Build a success response frame.
pub fn success_response(id: &str, payload: Value) -> OpenClawFrame {
    OpenClawFrame::Response(ResponseFrame {
        id: id.to_string(),
        ok: true,
        payload: Some(payload),
        error: None,
    })
}

/// Build an error response frame.
pub fn error_response(id: &str, code: &str, message: &str) -> OpenClawFrame {
    OpenClawFrame::Response(ResponseFrame {
        id: id.to_string(),
        ok: false,
        payload: None,
        error: Some(OpenClawError {
            code: code.to_string(),
            message: message.to_string(),
            data: None,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_frame_round_trip() {
        let frame = OpenClawFrame::Request(RequestFrame {
            id: "req-1".to_string(),
            method: "chat.send".to_string(),
            params: Some(serde_json::json!({"message": "hello"})),
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""type":"req"#));
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Request(r) => {
                assert_eq!(r.id, "req-1");
                assert_eq!(r.method, "chat.send");
            }
            _ => panic!("expected Request"),
        }
    }

    #[test]
    fn response_frame_success_round_trip() {
        let frame = OpenClawFrame::Response(ResponseFrame {
            id: "req-1".to_string(),
            ok: true,
            payload: Some(serde_json::json!({"status": "ok"})),
            error: None,
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""type":"res"#));
        assert!(json.contains(r#""ok":true"#));
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Response(r) => {
                assert_eq!(r.id, "req-1");
                assert!(r.ok);
                assert!(r.payload.is_some());
                assert!(r.error.is_none());
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn response_frame_error_round_trip() {
        let frame = OpenClawFrame::Response(ResponseFrame {
            id: "req-2".to_string(),
            ok: false,
            payload: None,
            error: Some(OpenClawError {
                code: "INVALID_REQUEST".to_string(),
                message: "not found".to_string(),
                data: None,
            }),
        });
        let json = serde_json::to_string(&frame).unwrap();
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Response(r) => {
                assert!(!r.ok);
                let err = r.error.unwrap();
                assert_eq!(err.code, "INVALID_REQUEST");
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn event_frame_round_trip() {
        let frame = OpenClawFrame::Event(EventFrame {
            event: "chat".to_string(),
            payload: serde_json::json!({"text": "hi"}),
            seq: Some(5),
            state_version: None,
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""type":"event"#));
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Event(e) => {
                assert_eq!(e.event, "chat");
                assert_eq!(e.seq, Some(5));
            }
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn challenge_event_construction() {
        let frame = challenge_event("abc-123", 1_700_000_000_000);
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""event":"connect.challenge"#));
        assert!(json.contains(r#""nonce":"abc-123"#));
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Event(e) => {
                assert_eq!(e.event, "connect.challenge");
                let cp: ChallengePayload = serde_json::from_value(e.payload).unwrap();
                assert_eq!(cp.nonce, "abc-123");
                assert_eq!(cp.ts, 1_700_000_000_000);
            }
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn connect_request_construction() {
        let params = ConnectParams {
            min_protocol: 3,
            max_protocol: 3,
            client: ClientInfo {
                id: "cli".to_string(),
                version: "0.62.1".to_string(),
                platform: "macos".to_string(),
                mode: "cli".to_string(),
                display_name: None,
                instance_id: None,
            },
            role: Some("operator".to_string()),
            scopes: Some(vec!["operator.read".to_string()]),
            caps: None,
            auth: Some(ConnectAuth {
                token: Some("tok".to_string()),
                device_token: None,
                password: None,
            }),
            device: None,
            locale: None,
            user_agent: None,
        };
        let frame = connect_request("hs-1", params).unwrap();
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""method":"connect"#));
        assert!(json.contains(r#""minProtocol":3"#));
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Request(r) => {
                assert_eq!(r.method, "connect");
                let cp: ConnectParams = serde_json::from_value(r.params.unwrap()).unwrap();
                assert_eq!(cp.client.id, "cli");
                assert_eq!(cp.min_protocol, 3);
            }
            _ => panic!("expected Request"),
        }
    }

    #[test]
    fn hello_ok_response_construction() {
        let frame = hello_ok_response("hs-1", 3, "conn-abc").unwrap();
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains(r#""ok":true"#));
        assert!(json.contains(r#""hello-ok"#));
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Response(r) => {
                assert!(r.ok);
                let ho: HelloOkPayload = serde_json::from_value(r.payload.unwrap()).unwrap();
                assert_eq!(ho.payload_type, "hello-ok");
                assert_eq!(ho.protocol, 3);
                assert_eq!(ho.server.unwrap().conn_id, "conn-abc");
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn success_response_construction() {
        let frame = success_response("r-1", serde_json::json!({"ok": true}));
        let json = serde_json::to_string(&frame).unwrap();
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Response(r) => {
                assert!(r.ok);
                assert_eq!(r.payload.unwrap()["ok"], true);
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn error_response_construction() {
        let frame = error_response("r-2", "NOT_FOUND", "missing");
        let json = serde_json::to_string(&frame).unwrap();
        let parsed: OpenClawFrame = serde_json::from_str(&json).unwrap();
        match parsed {
            OpenClawFrame::Response(r) => {
                assert!(!r.ok);
                assert_eq!(r.error.unwrap().code, "NOT_FOUND");
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn real_openclaw_challenge_parses() {
        let raw = r#"{"type":"event","event":"connect.challenge","payload":{"nonce":"b0681da8-78b8-47e2-b326-9425bf7fa9c3","ts":1771987648990}}"#;
        let parsed: OpenClawFrame = serde_json::from_str(raw).unwrap();
        match parsed {
            OpenClawFrame::Event(e) => {
                assert_eq!(e.event, "connect.challenge");
                let cp: ChallengePayload = serde_json::from_value(e.payload).unwrap();
                assert_eq!(cp.nonce, "b0681da8-78b8-47e2-b326-9425bf7fa9c3");
                assert_eq!(cp.ts, 1_771_987_648_990);
            }
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn real_openclaw_error_response_parses() {
        let raw = r#"{"type":"res","id":"chat-1","ok":false,"error":{"code":"INVALID_REQUEST","message":"missing scope: operator.write"}}"#;
        let parsed: OpenClawFrame = serde_json::from_str(raw).unwrap();
        match parsed {
            OpenClawFrame::Response(r) => {
                assert!(!r.ok);
                assert_eq!(r.error.unwrap().code, "INVALID_REQUEST");
            }
            _ => panic!("expected Response"),
        }
    }
}
