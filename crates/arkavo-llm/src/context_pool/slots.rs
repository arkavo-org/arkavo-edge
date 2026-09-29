//! A bounded set of reusable items with a waiting queue.
//!
//! Generic over the item so the bound, the wait and the timeout can be
//! tested without loading a model.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::Notify;

/// What a caller got when it asked for a slot.
#[derive(Debug)]
pub(super) enum Claim<T> {
    /// An idle item, now counted as in use.
    Idle(T),
    /// No idle item, but the bound leaves room for one more. The slot is
    /// already counted as in use; the caller builds the item, or calls
    /// [`Slots::forfeit`] if it cannot.
    Vacant,
    /// Every slot is in use.
    Exhausted,
}

struct State<T> {
    idle: VecDeque<T>,
    in_use: usize,
}

pub(super) struct Slots<T> {
    state: Mutex<State<T>>,
    freed: Notify,
    max: usize,
}

impl<T> Slots<T> {
    /// A bound of zero could never serve a request, so it is raised to one.
    pub(super) fn new(max: usize) -> Self {
        Self {
            state: Mutex::new(State {
                idle: VecDeque::new(),
                in_use: 0,
            }),
            freed: Notify::new(),
            max: max.max(1),
        }
    }

    pub(super) const fn max(&self) -> usize {
        self.max
    }

    /// Counts stay consistent even if a holder panicked, because every
    /// mutation below is a single push, pop or counter change.
    fn state(&self) -> std::sync::MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Take an idle item or reserve room for a new one, without waiting.
    pub(super) fn claim(&self) -> Claim<T> {
        let mut state = self.state();
        let claim = if let Some(item) = state.idle.pop_front() {
            Claim::Idle(item)
        } else if state.in_use < self.max {
            Claim::Vacant
        } else {
            return Claim::Exhausted;
        };
        state.in_use += 1;
        drop(state);
        claim
    }

    /// Reserve room for a new item only, leaving idle items where they are.
    /// Returns false when the bound is reached.
    pub(super) fn claim_vacant(&self) -> bool {
        let mut state = self.state();
        if state.in_use + state.idle.len() < self.max {
            state.in_use += 1;
            true
        } else {
            false
        }
    }

    /// Wait up to `limit` for a slot. `None` means the wait timed out.
    pub(super) async fn claim_within(&self, limit: Duration) -> Option<Claim<T>> {
        let wait = async {
            loop {
                let freed = self.freed.notified();
                tokio::pin!(freed);
                // Registered before the check, so a release that lands
                // between the check and the await still wakes this waiter.
                freed.as_mut().enable();
                match self.claim() {
                    Claim::Exhausted => freed.await,
                    claim => return claim,
                }
            }
        };
        tokio::time::timeout(limit, wait).await.ok()
    }

    /// Return an item so the next caller can reuse it.
    pub(super) fn give_back(&self, item: T) {
        {
            let mut state = self.state();
            state.in_use = state.in_use.saturating_sub(1);
            state.idle.push_back(item);
        }
        self.freed.notify_one();
    }

    /// Free a slot whose item was never built or must not be reused.
    pub(super) fn forfeit(&self) {
        {
            let mut state = self.state();
            state.in_use = state.in_use.saturating_sub(1);
        }
        self.freed.notify_one();
    }

    /// `(idle, in_use)`.
    pub(super) fn counts(&self) -> (usize, usize) {
        let state = self.state();
        (state.idle.len(), state.in_use)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Stands in for context creation and counts how many were ever built.
    fn build(built: &AtomicUsize) -> usize {
        built.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn take(slots: &Slots<usize>, built: &AtomicUsize) -> Option<usize> {
        match slots.claim() {
            Claim::Idle(item) => Some(item),
            Claim::Vacant => Some(build(built)),
            Claim::Exhausted => None,
        }
    }

    #[test]
    fn items_are_built_lazily_up_to_the_bound() {
        let slots = Slots::new(2);
        let built = AtomicUsize::new(0);
        assert_eq!(slots.counts(), (0, 0));

        let first = take(&slots, &built).expect("first slot");
        let second = take(&slots, &built).expect("second slot");
        assert_eq!((first, second), (1, 2));
        assert_eq!(slots.counts(), (0, 2));
    }

    /// Regression: an exhausted pool used to be answered by building a
    /// context outside it, so the bound did not bound anything.
    #[test]
    fn nothing_is_built_once_the_bound_is_reached() {
        let slots = Slots::new(1);
        let built = AtomicUsize::new(0);
        let held = take(&slots, &built).expect("only slot");

        assert!(take(&slots, &built).is_none());
        assert!(!slots.claim_vacant());
        assert_eq!(built.load(Ordering::SeqCst), 1);

        slots.give_back(held);
        assert_eq!(take(&slots, &built), Some(held));
        assert_eq!(built.load(Ordering::SeqCst), 1, "released item is reused");
    }

    #[test]
    fn a_zero_bound_still_serves_one() {
        let slots: Slots<usize> = Slots::new(0);
        assert_eq!(slots.max(), 1);
        assert!(matches!(slots.claim(), Claim::Vacant));
        assert!(matches!(slots.claim(), Claim::Exhausted));
    }

    #[test]
    fn a_forfeited_slot_can_be_claimed_again() {
        let slots: Slots<usize> = Slots::new(1);
        assert!(matches!(slots.claim(), Claim::Vacant));
        slots.forfeit();
        assert_eq!(slots.counts(), (0, 0));
        assert!(matches!(slots.claim(), Claim::Vacant));
    }

    #[tokio::test]
    async fn second_caller_waits_and_gets_the_released_item() {
        let slots = Arc::new(Slots::new(1));
        let built = Arc::new(AtomicUsize::new(0));
        let held = take(&slots, &built).expect("only slot");

        let waiter = {
            let slots = Arc::clone(&slots);
            tokio::spawn(async move { slots.claim_within(Duration::from_secs(5)).await })
        };

        // Long enough for the waiter to park; it must still be waiting.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !waiter.is_finished(),
            "waiter returned while the slot was held"
        );
        assert_eq!(slots.counts(), (0, 1));

        slots.give_back(held);
        let claim = waiter.await.expect("waiter task");
        assert!(matches!(claim, Some(Claim::Idle(item)) if item == held));
        assert_eq!(built.load(Ordering::SeqCst), 1, "waiting built nothing");
        assert_eq!(slots.counts(), (0, 1));
    }

    #[tokio::test]
    async fn waiting_gives_up_after_the_limit() {
        let slots = Slots::new(1);
        let built = AtomicUsize::new(0);
        let _held = take(&slots, &built).expect("only slot");

        let started = std::time::Instant::now();
        let claim = slots.claim_within(Duration::from_millis(60)).await;
        assert!(claim.is_none());
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert_eq!(slots.counts(), (0, 1), "a timed-out waiter holds nothing");
    }

    #[tokio::test]
    async fn every_waiter_is_served_in_turn() {
        let slots = Arc::new(Slots::new(1));
        let built = Arc::new(AtomicUsize::new(0));
        let served = Arc::new(AtomicUsize::new(0));
        let concurrent = Arc::new(AtomicUsize::new(0));

        let mut tasks = Vec::new();
        for _ in 0..4 {
            let slots = Arc::clone(&slots);
            let built = Arc::clone(&built);
            let served = Arc::clone(&served);
            let concurrent = Arc::clone(&concurrent);
            tasks.push(tokio::spawn(async move {
                let item = match slots.claim_within(Duration::from_secs(5)).await {
                    Some(Claim::Idle(item)) => item,
                    Some(Claim::Vacant) => build(&built),
                    Some(Claim::Exhausted) | None => panic!("waiter was not served"),
                };
                assert_eq!(concurrent.fetch_add(1, Ordering::SeqCst), 0);
                tokio::time::sleep(Duration::from_millis(10)).await;
                concurrent.fetch_sub(1, Ordering::SeqCst);
                served.fetch_add(1, Ordering::SeqCst);
                slots.give_back(item);
            }));
        }
        for task in tasks {
            task.await.expect("waiter task");
        }

        assert_eq!(served.load(Ordering::SeqCst), 4);
        assert_eq!(built.load(Ordering::SeqCst), 1);
        assert_eq!(slots.counts(), (1, 0));
    }

    #[tokio::test]
    async fn a_forfeit_wakes_a_waiter() {
        let slots: Arc<Slots<usize>> = Arc::new(Slots::new(1));
        assert!(matches!(slots.claim(), Claim::Vacant));

        let waiter = {
            let slots = Arc::clone(&slots);
            tokio::spawn(async move { slots.claim_within(Duration::from_secs(5)).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        slots.forfeit();

        let claim = waiter.await.expect("waiter task");
        assert!(matches!(claim, Some(Claim::Vacant)));
    }
}
