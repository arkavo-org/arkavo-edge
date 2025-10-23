use crate::error::Result;
use bytes::{Buf, BufMut, BytesMut};

pub struct Protocol;

impl Protocol {
    pub fn serialize_command(
        id: u32,
        op: u8,
        selector: &str,
        payload: &str,
        property: Option<&str>,
    ) -> Result<Vec<u8>> {
        let mut buf = BytesMut::with_capacity(256);

        buf.put_u32_le(id);
        buf.put_u8(op);

        buf.put_u32_le(selector.len() as u32);
        buf.put_slice(selector.as_bytes());

        buf.put_u32_le(payload.len() as u32);
        buf.put_slice(payload.as_bytes());

        if let Some(prop) = property {
            buf.put_u32_le(prop.len() as u32);
            buf.put_slice(prop.as_bytes());
        } else {
            buf.put_u32_le(0);
        }

        Ok(buf.to_vec())
    }

    pub fn deserialize_feedback(_data: &[u8]) -> Result<DOMFeedbackSimple> {
        Ok(DOMFeedbackSimple {
            id: 0,
            status: 0,
            exec_time_ns: 0,
            message: String::new(),
        })
    }

    pub fn frame_message(data: &[u8]) -> Vec<u8> {
        let len = data.len() as u32;
        let mut framed = BytesMut::with_capacity(4 + data.len());
        framed.put_u32_le(len);
        framed.put_slice(data);
        framed.to_vec()
    }

    pub fn unframe_message(buf: &mut BytesMut) -> Result<Option<Vec<u8>>> {
        if buf.len() < 4 {
            return Ok(None);
        }

        let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;

        if buf.len() < 4 + len {
            return Ok(None);
        }

        buf.advance(4);
        let data = buf.split_to(len).to_vec();
        Ok(Some(data))
    }
}

#[derive(Debug, Clone)]
pub struct DOMFeedbackSimple {
    pub id: u32,
    pub status: u8,
    pub exec_time_ns: u64,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct DOMEvent {
    pub event_type: String,
    pub selector: String,
    pub target_id: String,
    pub value: String,
    pub data: String,
}

#[derive(Debug, Clone)]
pub struct DOMError {
    pub error_type: String,
    pub severity: String,
    pub message: String,
    pub source: String,
    pub line: u32,
}

impl DOMError {
    pub fn format_for_llm(&self) -> String {
        format!("{}:{} - {}", self.source, self.line, self.message)
    }
}

impl Protocol {
    pub fn deserialize_event(data: &[u8]) -> Result<DOMEvent> {
        if data.is_empty() || data[0] != 0x02 {
            return Err(crate::error::CefError::ProtocolError(
                "Invalid event message type".to_string(),
            ));
        }

        let mut cursor = 1;

        let read_string = |cursor: &mut usize| -> Result<String> {
            if data.len() < *cursor + 4 {
                return Err(crate::error::CefError::ProtocolError(
                    "Incomplete string length".to_string(),
                ));
            }
            let len = u32::from_le_bytes([
                data[*cursor],
                data[*cursor + 1],
                data[*cursor + 2],
                data[*cursor + 3],
            ]) as usize;
            *cursor += 4;

            if data.len() < *cursor + len {
                return Err(crate::error::CefError::ProtocolError(
                    "Incomplete string data".to_string(),
                ));
            }
            let s = String::from_utf8_lossy(&data[*cursor..*cursor + len]).to_string();
            *cursor += len;
            Ok(s)
        };

        let event_type = read_string(&mut cursor)?;
        let selector = read_string(&mut cursor)?;
        let target_id = read_string(&mut cursor)?;
        let value = read_string(&mut cursor)?;
        let data_field = read_string(&mut cursor)?;

        Ok(DOMEvent {
            event_type,
            selector,
            target_id,
            value,
            data: data_field,
        })
    }

    pub fn deserialize_error(data: &[u8]) -> Result<DOMError> {
        if data.is_empty() || data[0] != 0x03 {
            return Err(crate::error::CefError::ProtocolError(
                "Invalid error message type".to_string(),
            ));
        }

        let mut cursor = 1;

        let read_string = |cursor: &mut usize| -> Result<String> {
            if data.len() < *cursor + 4 {
                return Err(crate::error::CefError::ProtocolError(
                    "Incomplete string length".to_string(),
                ));
            }
            let len = u32::from_le_bytes([
                data[*cursor],
                data[*cursor + 1],
                data[*cursor + 2],
                data[*cursor + 3],
            ]) as usize;
            *cursor += 4;

            if data.len() < *cursor + len {
                return Err(crate::error::CefError::ProtocolError(
                    "Incomplete string data".to_string(),
                ));
            }
            let s = String::from_utf8_lossy(&data[*cursor..*cursor + len]).to_string();
            *cursor += len;
            Ok(s)
        };

        let error_type = read_string(&mut cursor)?;
        let severity = read_string(&mut cursor)?;
        let message = read_string(&mut cursor)?;
        let source = read_string(&mut cursor)?;

        let line = if data.len() >= cursor + 4 {
            let l = u32::from_le_bytes([
                data[cursor],
                data[cursor + 1],
                data[cursor + 2],
                data[cursor + 3],
            ]);
            l
        } else {
            0
        };

        Ok(DOMError {
            error_type,
            severity,
            message,
            source,
            line,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_command() {
        let data = Protocol::serialize_command(1, 0, "#content", "<div>Hello</div>", None);
        assert!(data.is_ok());
    }

    #[test]
    fn test_frame_unframe() {
        let message = b"test message";
        let framed = Protocol::frame_message(message);

        let mut buf = BytesMut::from(&framed[..]);
        let unframed = Protocol::unframe_message(&mut buf).unwrap();

        assert_eq!(unframed, Some(message.to_vec()));
    }
}
