use crate::error::Result;
use crate::uds::UdsTransport;
use std::sync::atomic::{AtomicU32, Ordering};

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum DOMOp {
    ReplaceInnerHTML = 0,
    SetAttribute = 1,
    SetStyle = 2,
    RemoveNode = 3,
    AppendNode = 4,
    QuerySelector = 5,
    AddEventListener = 6,
    SetTextContent = 7,
}

pub struct DOMCommandBuilder {
    transport: UdsTransport,
    next_id: AtomicU32,
}

impl DOMCommandBuilder {
    pub fn new(transport: UdsTransport) -> Self {
        Self {
            transport,
            next_id: AtomicU32::new(0),
        }
    }

    fn next_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn replace_inner_html(&mut self, selector: &str, html: &str) -> Result<()> {
        let id = self.next_id();
        self.transport
            .send_command(id, DOMOp::ReplaceInnerHTML as u8, selector, html, None)
            .await?;

        let feedback = self.transport.recv_feedback().await?;
        if feedback.status != 0 {
            return Err(crate::error::CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }

    pub async fn set_attribute(
        &mut self,
        selector: &str,
        attribute: &str,
        value: &str,
    ) -> Result<()> {
        let id = self.next_id();
        self.transport
            .send_command(
                id,
                DOMOp::SetAttribute as u8,
                selector,
                value,
                Some(attribute),
            )
            .await?;

        let feedback = self.transport.recv_feedback().await?;
        if feedback.status != 0 {
            return Err(crate::error::CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }

    pub async fn set_style(&mut self, selector: &str, property: &str, value: &str) -> Result<()> {
        let id = self.next_id();
        self.transport
            .send_command(id, DOMOp::SetStyle as u8, selector, value, Some(property))
            .await?;

        let feedback = self.transport.recv_feedback().await?;
        if feedback.status != 0 {
            return Err(crate::error::CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }

    pub async fn set_text_content(&mut self, selector: &str, text: &str) -> Result<()> {
        let id = self.next_id();
        self.transport
            .send_command(id, DOMOp::SetTextContent as u8, selector, text, None)
            .await?;

        let feedback = self.transport.recv_feedback().await?;
        if feedback.status != 0 {
            return Err(crate::error::CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }

    pub async fn remove_node(&mut self, selector: &str) -> Result<()> {
        let id = self.next_id();
        self.transport
            .send_command(id, DOMOp::RemoveNode as u8, selector, "", None)
            .await?;

        let feedback = self.transport.recv_feedback().await?;
        if feedback.status != 0 {
            return Err(crate::error::CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }

    pub async fn add_event_listener(&mut self, selector: &str, event_type: &str) -> Result<()> {
        let id = self.next_id();
        self.transport
            .send_command(
                id,
                DOMOp::AddEventListener as u8,
                selector,
                event_type,
                None,
            )
            .await?;

        let feedback = self.transport.recv_feedback().await?;
        if feedback.status != 0 {
            return Err(crate::error::CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }
}
