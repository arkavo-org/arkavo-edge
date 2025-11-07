use crate::command_tracker::CommandTracker;
use crate::error::{CefError, Result};
use crate::uds::UdsTransport;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::Semaphore;

const MAX_HTML_PAYLOAD_SIZE: usize = 1_048_576;
const MAX_CSS_PAYLOAD_SIZE: usize = 102_400;
const MAX_CONCURRENT_COMMANDS: usize = 100;

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
    rate_limiter: Arc<Semaphore>,
    tracker: Arc<CommandTracker>,
}

impl DOMCommandBuilder {
    pub fn new(transport: UdsTransport) -> Self {
        Self {
            transport,
            next_id: AtomicU32::new(0),
            rate_limiter: Arc::new(Semaphore::new(MAX_CONCURRENT_COMMANDS)),
            tracker: Arc::new(CommandTracker::new()),
        }
    }

    pub fn with_tracker(transport: UdsTransport, tracker: Arc<CommandTracker>) -> Self {
        Self {
            transport,
            next_id: AtomicU32::new(0),
            rate_limiter: Arc::new(Semaphore::new(MAX_CONCURRENT_COMMANDS)),
            tracker,
        }
    }

    pub fn tracker(&self) -> Arc<CommandTracker> {
        self.tracker.clone()
    }

    /// Attempts to receive an event from the transport (non-blocking).
    pub async fn try_recv_event(&mut self) -> crate::Result<Option<crate::protocol::DOMEvent>> {
        match self.transport.try_recv_message().await? {
            Some(crate::uds::ReceivedMessage::Event(event)) => Ok(Some(event)),
            Some(other_msg) => {
                // Re-queue non-event messages so they can be retrieved by try_recv_message
                self.transport.requeue_message(other_msg);
                Ok(None)
            }
            None => Ok(None),
        }
    }

    /// Attempts to receive an error from the transport (non-blocking).
    pub async fn try_recv_error(&mut self) -> crate::Result<Option<crate::protocol::DOMError>> {
        match self.transport.try_recv_message().await? {
            Some(crate::uds::ReceivedMessage::Error(error)) => Ok(Some(error)),
            Some(other_msg) => {
                // Re-queue non-error messages so they can be retrieved by try_recv_message
                self.transport.requeue_message(other_msg);
                Ok(None)
            }
            None => Ok(None),
        }
    }

    /// Attempts to receive any message (feedback, event, or error) from the transport (non-blocking).
    pub async fn try_recv_message(&mut self) -> crate::Result<Option<crate::uds::ReceivedMessage>> {
        self.transport.try_recv_message().await
    }

    fn next_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    fn validate_html_size(&self, html: &str) -> Result<()> {
        if html.len() > MAX_HTML_PAYLOAD_SIZE {
            return Err(CefError::DomCommandFailed(format!(
                "HTML payload too large: {} bytes (max: {} bytes)",
                html.len(),
                MAX_HTML_PAYLOAD_SIZE
            )));
        }
        Ok(())
    }

    fn validate_css_size(&self, css: &str) -> Result<()> {
        if css.len() > MAX_CSS_PAYLOAD_SIZE {
            return Err(CefError::DomCommandFailed(format!(
                "CSS payload too large: {} bytes (max: {} bytes)",
                css.len(),
                MAX_CSS_PAYLOAD_SIZE
            )));
        }
        Ok(())
    }

    /// Replaces the inner HTML of an element matching the selector.
    ///
    /// # Panics
    /// Panics if the internal rate limiter semaphore is closed, which should never happen in normal operation.
    pub async fn replace_inner_html(&mut self, selector: &str, html: &str) -> Result<()> {
        println!(
            "[DEBUG] replace_inner_html called: selector='{}', html_len={}",
            selector,
            html.len()
        );

        self.validate_html_size(html)?;
        println!("[DEBUG] HTML size validation passed");

        let _permit = self
            .rate_limiter
            .acquire()
            .await
            .expect("Semaphore should never be closed");
        println!("[DEBUG] Rate limiter permit acquired");

        let id = self.next_id();
        println!("[DEBUG] Sending command with id={id}");

        self.tracker
            .track_command(id, DOMOp::ReplaceInnerHTML as u8, selector.to_string())
            .await;

        self.transport
            .send_command(id, DOMOp::ReplaceInnerHTML as u8, selector, html, None)
            .await?;
        println!("[DEBUG] Command sent, waiting for feedback");

        let feedback = self.transport.recv_feedback().await?;
        println!(
            "[DEBUG] Received feedback: status={}, message='{}'",
            feedback.status, feedback.message
        );

        self.tracker.complete_command(id).await;

        if feedback.status != 0 {
            eprintln!(
                "[ERROR] DOM command failed with status {}: {}",
                feedback.status, feedback.message
            );
            return Err(CefError::DomCommandFailed(feedback.message));
        }

        println!("[DEBUG] replace_inner_html completed successfully");
        Ok(())
    }

    /// Sets an attribute on an element matching the selector.
    ///
    /// # Panics
    /// Panics if the internal rate limiter semaphore is closed, which should never happen in normal operation.
    pub async fn set_attribute(
        &mut self,
        selector: &str,
        attribute: &str,
        value: &str,
    ) -> Result<()> {
        self.validate_html_size(value)?;

        let _permit = self
            .rate_limiter
            .acquire()
            .await
            .expect("Semaphore should never be closed");

        let id = self.next_id();
        self.tracker
            .track_command(id, DOMOp::SetAttribute as u8, selector.to_string())
            .await;

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
        self.tracker.complete_command(id).await;

        if feedback.status != 0 {
            return Err(CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }

    /// Sets a CSS style property on an element matching the selector.
    ///
    /// # Panics
    /// Panics if the internal rate limiter semaphore is closed, which should never happen in normal operation.
    pub async fn set_style(&mut self, selector: &str, property: &str, value: &str) -> Result<()> {
        self.validate_css_size(value)?;

        let _permit = self
            .rate_limiter
            .acquire()
            .await
            .expect("Semaphore should never be closed");

        let id = self.next_id();
        self.tracker
            .track_command(id, DOMOp::SetStyle as u8, selector.to_string())
            .await;

        self.transport
            .send_command(id, DOMOp::SetStyle as u8, selector, value, Some(property))
            .await?;

        let feedback = self.transport.recv_feedback().await?;
        self.tracker.complete_command(id).await;

        if feedback.status != 0 {
            return Err(CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }

    /// Sets the text content of an element matching the selector.
    ///
    /// # Panics
    /// Panics if the internal rate limiter semaphore is closed, which should never happen in normal operation.
    pub async fn set_text_content(&mut self, selector: &str, text: &str) -> Result<()> {
        self.validate_html_size(text)?;

        let _permit = self
            .rate_limiter
            .acquire()
            .await
            .expect("Semaphore should never be closed");

        let id = self.next_id();
        self.tracker
            .track_command(id, DOMOp::SetTextContent as u8, selector.to_string())
            .await;

        self.transport
            .send_command(id, DOMOp::SetTextContent as u8, selector, text, None)
            .await?;

        let feedback = self.transport.recv_feedback().await?;
        self.tracker.complete_command(id).await;

        if feedback.status != 0 {
            return Err(CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }

    /// Removes a node matching the selector from the DOM.
    ///
    /// # Panics
    /// Panics if the internal rate limiter semaphore is closed, which should never happen in normal operation.
    pub async fn remove_node(&mut self, selector: &str) -> Result<()> {
        let _permit = self
            .rate_limiter
            .acquire()
            .await
            .expect("Semaphore should never be closed");

        let id = self.next_id();
        self.tracker
            .track_command(id, DOMOp::RemoveNode as u8, selector.to_string())
            .await;

        self.transport
            .send_command(id, DOMOp::RemoveNode as u8, selector, "", None)
            .await?;

        let feedback = self.transport.recv_feedback().await?;
        self.tracker.complete_command(id).await;

        if feedback.status != 0 {
            return Err(CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }

    /// Adds an event listener to an element matching the selector.
    ///
    /// # Panics
    /// Panics if the internal rate limiter semaphore is closed, which should never happen in normal operation.
    pub async fn add_event_listener(&mut self, selector: &str, event_type: &str) -> Result<()> {
        let _permit = self
            .rate_limiter
            .acquire()
            .await
            .expect("Semaphore should never be closed");

        let id = self.next_id();
        self.tracker
            .track_command(id, DOMOp::AddEventListener as u8, selector.to_string())
            .await;

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
        self.tracker.complete_command(id).await;

        if feedback.status != 0 {
            return Err(CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }
}
