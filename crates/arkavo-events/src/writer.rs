use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, mpsc};
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

    fn make_event(seq: u64) -> Event {
        Event::new(
            "test-session".to_string(),
            seq,
            "test-agent".to_string(),
            EventPayload::ReasoningStep {
                step_type: "test".to_string(),
                description: format!("Step {seq}"),
                metadata: None,
            },
        )
    }

    // Covers EVENT-005: "Events serialized, written to configured sink"
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

        for i in 0..25 {
            writer.write(make_event(i)).await.unwrap();
        }

        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 25);
    }

    // Covers EVENT-005: "Batch writes supported" — flush triggers at batch_size
    #[tokio::test]
    async fn test_batch_flush_triggers_at_batch_size() {
        let flush_count = Arc::new(AtomicUsize::new(0));
        let flush_count_clone = flush_count.clone();

        let writer = EventWriterBuilder::new()
            .with_config(EventWriterConfig {
                buffer_size: 1000,
                flush_interval: Duration::from_secs(60), // Long interval so only batch triggers
                batch_size: 5,
            })
            .add_handler(move |_events| {
                flush_count_clone.fetch_add(1, Ordering::SeqCst);
            })
            .build();

        // Write exactly batch_size events
        for i in 0..5 {
            writer.write(make_event(i)).await.unwrap();
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
        // Should have flushed once when batch_size reached
        assert!(flush_count.load(Ordering::SeqCst) >= 1);
    }

    // Covers EVENT-005: Multiple batches accumulate correctly
    #[tokio::test]
    async fn test_multiple_batches_all_delivered() {
        let total = Arc::new(AtomicUsize::new(0));
        let total_clone = total.clone();

        let writer = EventWriterBuilder::new()
            .with_config(EventWriterConfig {
                buffer_size: 1000,
                flush_interval: Duration::from_millis(20),
                batch_size: 10,
            })
            .add_handler(move |events| {
                total_clone.fetch_add(events.len(), Ordering::SeqCst);
            })
            .build();

        for i in 0..50 {
            writer.write(make_event(i)).await.unwrap();
        }

        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(total.load(Ordering::SeqCst), 50);
    }
}
