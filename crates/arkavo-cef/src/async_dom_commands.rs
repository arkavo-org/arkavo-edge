use crate::async_transport::AsyncTransport;
use crate::command_tracker::CommandTracker;
use crate::error::{CefError, Result};
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

pub struct AsyncDOMCommandBuilder {
    transport: AsyncTransport,
    next_id: AtomicU32,
    rate_limiter: Arc<Semaphore>,
    tracker: Arc<CommandTracker>,
}

impl AsyncDOMCommandBuilder {
    pub fn new(transport: AsyncTransport) -> Self {
        Self {
            transport,
            next_id: AtomicU32::new(0),
            rate_limiter: Arc::new(Semaphore::new(MAX_CONCURRENT_COMMANDS)),
            tracker: Arc::new(CommandTracker::new()),
        }
    }

    pub fn with_tracker(transport: AsyncTransport, tracker: Arc<CommandTracker>) -> Self {
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

    /// # Panics
    /// Panics if the internal rate limiter semaphore is closed (should never happen).
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

        let feedback_rx = self
            .transport
            .send_command(id, DOMOp::ReplaceInnerHTML as u8, selector, html, None)
            .await?;
        println!("[DEBUG] Command sent, waiting for feedback (non-blocking, 10s timeout)");

        let feedback = tokio::time::timeout(tokio::time::Duration::from_secs(10), feedback_rx)
            .await
            .map_err(|_| {
                eprintln!("[ERROR] Timeout waiting for feedback on command {id}");
                CefError::Timeout
            })?
            .map_err(|_| CefError::DomCommandFailed("Feedback channel closed".to_string()))?;
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

    /// # Panics
    /// Panics if the internal rate limiter semaphore is closed (should never happen).
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

        let feedback_rx = self
            .transport
            .send_command(
                id,
                DOMOp::SetAttribute as u8,
                selector,
                value,
                Some(attribute),
            )
            .await?;

        let feedback = feedback_rx
            .await
            .map_err(|_| CefError::DomCommandFailed("Feedback channel closed".to_string()))?;

        self.tracker.complete_command(id).await;

        if feedback.status != 0 {
            return Err(CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }

    /// # Panics
    /// Panics if the internal rate limiter semaphore is closed (should never happen).
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

        let feedback_rx = self
            .transport
            .send_command(id, DOMOp::SetStyle as u8, selector, value, Some(property))
            .await?;

        let feedback = feedback_rx
            .await
            .map_err(|_| CefError::DomCommandFailed("Feedback channel closed".to_string()))?;

        self.tracker.complete_command(id).await;

        if feedback.status != 0 {
            return Err(CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }

    /// # Panics
    /// Panics if the internal rate limiter semaphore is closed (should never happen).
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

        let feedback_rx = self
            .transport
            .send_command(
                id,
                DOMOp::AddEventListener as u8,
                selector,
                event_type,
                None,
            )
            .await?;

        let feedback = feedback_rx
            .await
            .map_err(|_| CefError::DomCommandFailed("Feedback channel closed".to_string()))?;

        self.tracker.complete_command(id).await;

        if feedback.status != 0 {
            return Err(CefError::DomCommandFailed(feedback.message));
        }

        Ok(())
    }

    pub fn try_recv_event(&mut self) -> Option<crate::DOMEvent> {
        self.transport.try_recv_event()
    }
}
