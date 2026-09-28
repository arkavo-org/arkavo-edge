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
                result = receiver.recv() => {
                    match result {
                        Some(event) => {
                            let mut state_guard = state.lock().await;
                            state_guard.buffer.push_back(event);

                            if state_guard.buffer.len() >= config.batch_size {
                                Self::flush_buffer(&mut state_guard, config.batch_size).await;
                            }
                        }
                        None => {
                            // Sender dropped — drain buffer
                            let mut state_guard = state.lock().await;
                            while !state_guard.buffer.is_empty() {
                                Self::flush_buffer(&mut state_guard, config.batch_size).await;
                            }
                            break;
                        }
                    }
                }
                _ = flush_interval.tick() => {
                    let mut state_guard = state.lock().await;
                    if !state_guard.buffer.is_empty() &&
                       state_guard.last_flush.elapsed() >= config.flush_interval {
                        Self::flush_buffer(&mut state_guard, config.batch_size).await;
                    }
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
    use arkavo_test_macros::spec;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Delivery happens on the writer's spawned task, so a fixed sleep before
    /// asserting races a loaded scheduler. Polling until the condition holds,
    /// with a deadline far past any healthy delivery, removes that race while
    /// a lost event still fails the test.
    async fn wait_until(condition: impl Fn() -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !condition() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

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

    #[spec("EVENT-005")]
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

        wait_until(|| counter.load(Ordering::SeqCst) >= 25).await;
        assert_eq!(counter.load(Ordering::SeqCst), 25);
    }

    #[spec("EVENT-005")]
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

        // Should have flushed once when batch_size reached; the 60s interval
        // cannot fire inside the wait's deadline, so only the batch can.
        wait_until(|| flush_count.load(Ordering::SeqCst) >= 1).await;
        assert_eq!(flush_count.load(Ordering::SeqCst), 1);
    }

    #[spec("EVENT-005")]
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

        wait_until(|| total.load(Ordering::SeqCst) >= 50).await;
        assert_eq!(total.load(Ordering::SeqCst), 50);
    }

    #[spec("EVENT-005")]
    #[tokio::test]
    async fn test_write_returns_buffer_full_after_abort() {
        let writer = EventWriterBuilder::new()
            .with_config(EventWriterConfig {
                buffer_size: 10,
                flush_interval: Duration::from_millis(50),
                batch_size: 5,
            })
            .build();

        // Abort the writer loop so the receiver is dropped. Waiting on the
        // channel's own close signal, rather than a sleep, guarantees the
        // aborted task has actually released the receiver.
        writer._handle.abort();
        tokio::time::timeout(Duration::from_secs(5), writer.sender.closed())
            .await
            .expect("aborted writer loop never dropped its receiver");

        // Write should now fail with BufferFull (receiver gone)
        let result = writer.write(make_event(0)).await;
        assert!(
            matches!(result, Err(crate::EventError::BufferFull)),
            "Write after abort should return BufferFull, got: {result:?}"
        );
    }

    #[spec("EVENT-005")]
    #[tokio::test]
    async fn test_final_flush_on_sender_drop() {
        let total = Arc::new(AtomicUsize::new(0));
        let total_clone = total.clone();

        let writer = EventWriterBuilder::new()
            .with_config(EventWriterConfig {
                buffer_size: 1000,
                flush_interval: Duration::from_secs(60), // Timer won't fire
                batch_size: 1000,                        // Won't reach batch threshold
            })
            .add_handler(move |events| {
                total_clone.fetch_add(events.len(), Ordering::SeqCst);
            })
            .build();

        // Write 3 events — neither batch_size nor timer can flush these
        for i in 0..3 {
            writer.write(make_event(i)).await.unwrap();
        }

        // Give writer loop time to recv events from channel into its buffer
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Verify nothing flushed yet (timer hasn't fired, batch not full)
        assert_eq!(
            total.load(Ordering::SeqCst),
            0,
            "Nothing should be flushed before drop"
        );

        // Drop the sender so writer_loop takes its `None` branch, then join the
        // loop: once it has exited, the final flush has run, with no timing
        // guess involved.
        let EventWriter {
            sender,
            _handle: writer_loop,
        } = writer;
        drop(sender);
        tokio::time::timeout(Duration::from_secs(5), writer_loop)
            .await
            .expect("writer loop did not exit after its sender dropped")
            .expect("writer loop panicked");

        assert_eq!(
            total.load(Ordering::SeqCst),
            3,
            "Only the else-branch final flush could have delivered these events"
        );
    }

    /// Second test for EVENT-005: Event writer with multiple handlers
    #[spec("EVENT-005")]
    #[tokio::test]
    async fn test_event_writer_multiple_handlers() {
        let counter1 = Arc::new(AtomicUsize::new(0));
        let counter2 = Arc::new(AtomicUsize::new(0));
        let c1 = counter1.clone();
        let c2 = counter2.clone();

        let writer = EventWriterBuilder::new()
            .with_config(EventWriterConfig {
                buffer_size: 1000,
                flush_interval: Duration::from_millis(50),
                batch_size: 10,
            })
            .add_handler(move |events| {
                c1.fetch_add(events.len(), Ordering::SeqCst);
            })
            .add_handler(move |events| {
                c2.fetch_add(events.len(), Ordering::SeqCst);
            })
            .build();

        for i in 0..20 {
            writer.write(make_event(i)).await.unwrap();
        }

        wait_until(|| {
            counter1.load(Ordering::SeqCst) >= 20 && counter2.load(Ordering::SeqCst) >= 20
        })
        .await;

        // Both handlers should receive all events
        assert_eq!(counter1.load(Ordering::SeqCst), 20);
        assert_eq!(counter2.load(Ordering::SeqCst), 20);
    }
}
