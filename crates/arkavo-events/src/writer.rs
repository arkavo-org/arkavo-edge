use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};
use tokio::time::interval;

use crate::{Event, EventError};

pub type EventHandler = Arc<dyn Fn(Vec<Event>) + Send + Sync>;

#[derive(Clone)]
pub struct EventWriterConfig {
    pub buffer_size: usize,
    pub flush_interval: Duration,
    pub batch_size: usize,
}

impl Default for EventWriterConfig {
    fn default() -> Self {
        Self {
            buffer_size: 10_000,
            flush_interval: Duration::from_millis(100),
            batch_size: 200,
        }
    }
}

pub struct EventWriter {
    sender: mpsc::Sender<Event>,
    _handle: tokio::task::JoinHandle<()>,
}

struct WriterState {
    buffer: VecDeque<Event>,
    last_flush: Instant,
    handlers: Vec<EventHandler>,
}

impl EventWriter {
    pub fn new(config: EventWriterConfig) -> Self {
        let (sender, receiver) = mpsc::channel(config.buffer_size);
        let state = Arc::new(Mutex::new(WriterState {
            buffer: VecDeque::with_capacity(config.buffer_size),
            last_flush: Instant::now(),
            handlers: Vec::new(),
        }));

        let handle = tokio::spawn(Self::writer_loop(receiver, state.clone(), config));

        Self {
            sender,
            _handle: handle,
        }
    }

    pub async fn write(&self, event: Event) -> Result<(), EventError> {
        self.sender
            .send(event)
            .await
            .map_err(|_| EventError::BufferFull)
    }

    pub async fn add_handler<F>(&self, _handler: F)
    where
        F: Fn(Vec<Event>) + Send + Sync + 'static,
    {
        // This would need access to the state to add handlers
        // For now, handlers would be configured at creation time
    }

    async fn writer_loop(
        mut receiver: mpsc::Receiver<Event>,
        state: Arc<Mutex<WriterState>>,
        config: EventWriterConfig,
    ) {
        let mut flush_interval = interval(config.flush_interval);

        loop {
            tokio::select! {
                Some(event) = receiver.recv() => {
                    let mut state_guard = state.lock().await;
                    state_guard.buffer.push_back(event);

                    if state_guard.buffer.len() >= config.batch_size {
                        Self::flush_buffer(&mut state_guard, config.batch_size).await;
                    }
                }
                _ = flush_interval.tick() => {
                    let mut state_guard = state.lock().await;
                    if !state_guard.buffer.is_empty() &&
                       state_guard.last_flush.elapsed() >= config.flush_interval {
                        Self::flush_buffer(&mut state_guard, config.batch_size).await;
                    }
                }
                else => {
                    // Receiver closed, do final flush
                    let mut state_guard = state.lock().await;
                    while !state_guard.buffer.is_empty() {
                        Self::flush_buffer(&mut state_guard, config.batch_size).await;
                    }
                    break;
                }
            }
        }
    }

    async fn flush_buffer(state: &mut WriterState, batch_size: usize) {
        let events: Vec<Event> = state
            .buffer
            .drain(..batch_size.min(state.buffer.len()))
            .collect();

        if !events.is_empty() {
            for handler in &state.handlers {
                handler(events.clone());
            }
            state.last_flush = Instant::now();
        }
    }
}

pub struct EventWriterBuilder {
    config: EventWriterConfig,
    handlers: Vec<EventHandler>,
}

impl Default for EventWriterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl EventWriterBuilder {
    pub fn new() -> Self {
        Self {
            config: EventWriterConfig::default(),
            handlers: Vec::new(),
        }
    }

    pub fn with_config(mut self, config: EventWriterConfig) -> Self {
        self.config = config;
        self
    }

    pub fn add_handler<F>(mut self, handler: F) -> Self
    where
        F: Fn(Vec<Event>) + Send + Sync + 'static,
    {
        self.handlers.push(Arc::new(handler));
        self
    }

    pub fn build(self) -> EventWriter {
        let (sender, receiver) = mpsc::channel(self.config.buffer_size);
        let state = Arc::new(Mutex::new(WriterState {
            buffer: VecDeque::with_capacity(self.config.buffer_size),
            last_flush: Instant::now(),
            handlers: self.handlers,
        }));

        let handle = tokio::spawn(EventWriter::writer_loop(
            receiver,
            state.clone(),
            self.config.clone(),
        ));

        EventWriter {
            sender,
            _handle: handle,
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;
    use crate::EventPayload;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_event_writer_basic() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let writer = EventWriterBuilder::new()
            .with_config(EventWriterConfig {
                buffer_size: 1000,
                flush_interval: Duration::from_millis(50),
                batch_size: 10,
            })
            .add_handler(move |events| {
                counter_clone.fetch_add(events.len(), Ordering::SeqCst);
            })
            .build();

        // Write some events
        for i in 0..25 {
            let event = Event::new(
                "test-session".to_string(),
                i,
                "test-agent".to_string(),
                EventPayload::ReasoningStep {
                    step_type: "test".to_string(),
                    description: format!("Step {i}"),
                    metadata: None,
                },
            );
            writer.write(event).await.unwrap();
        }

        // Wait for flush
        tokio::time::sleep(Duration::from_millis(150)).await;

        assert_eq!(counter.load(Ordering::SeqCst), 25);
    }
}
