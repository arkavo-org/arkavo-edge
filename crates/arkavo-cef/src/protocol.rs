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
