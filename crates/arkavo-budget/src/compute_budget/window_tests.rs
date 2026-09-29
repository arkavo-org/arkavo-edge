//! What a budget holds as its windows end, on a clock the test moves.

use super::*;
use std::sync::Mutex;

/// A clock that stands still until it is told how much time has passed.
#[derive(Clone)]
struct Elapsed(Arc<Mutex<Duration>>);

impl Elapsed {
    fn none() -> Self {
        Self(Arc::new(Mutex::new(Duration::ZERO)))
    }

    fn clock(&self) -> BudgetClock {
        let start = Instant::now();
        let elapsed = self.0.clone();
        BudgetClock::new(move || start + *elapsed.lock().expect("elapsed time"))
    }

    fn pass(&self, seconds: u64) {
        *self.0.lock().expect("elapsed time") += Duration::from_secs(seconds);
    }
}

fn own() -> BudgetAllocation {
    BudgetAllocation::self_managed()
}

fn self_managed(time: &Elapsed) -> AgentComputeBudget {
    AgentComputeBudget::self_managed(own(), time.clock())
}

fn spend_every_inference(budget: &mut AgentComputeBudget) {
    for _ in 0..own().max_inferences {
        budget.consume_inference(10, 0.0);
    }
}

/// Regression: nothing started a window after the first, so an agent that
/// had been up for ten minutes refused every request that carried no
/// allocation with "compute budget exhausted", and went on refusing.
#[test]
fn an_idle_agent_has_a_full_window_after_the_last_one_ended() {
    let time = Elapsed::none();
    let budget = self_managed(&time);

    time.pass(own().ttl_secs + 1);

    assert!(budget.has_remaining());
    assert_eq!(budget.status_label(), "active");
    let snapshot = budget.snapshot();
    assert!(snapshot.has_remaining);
    assert_eq!(snapshot.status, "active");
    assert_eq!(snapshot.remaining_inferences, own().max_inferences);
    assert_eq!(snapshot.remaining_tokens, own().max_tokens);
    assert!((snapshot.ttl_remaining_secs - own().ttl_secs as f64).abs() < f64::EPSILON);
}

#[test]
fn every_window_that_ends_is_followed_by_another() {
    let time = Elapsed::none();
    let mut budget = self_managed(&time);

    for _ in 0..3 {
        spend_every_inference(&mut budget);
        assert!(!budget.has_remaining());
        time.pass(own().ttl_secs);
        assert!(budget.has_remaining());
    }
}

/// Regression: a budget with no inferences left was refilled on the spot, so
/// it limited nothing.
#[test]
fn a_spent_window_stays_spent_until_it_ends() {
    let time = Elapsed::none();
    let mut budget = self_managed(&time);
    spend_every_inference(&mut budget);

    time.pass(own().ttl_secs - 1);

    assert!(!budget.has_remaining());
    assert!(!budget.renew_if_expired());
    assert_eq!(budget.remaining_inferences, 0);
    let snapshot = budget.snapshot();
    assert!(!snapshot.has_remaining);
    assert_eq!(snapshot.remaining_inferences, 0);
    assert_eq!(snapshot.status, "passive");

    time.pass(1);

    assert!(budget.has_remaining());
    assert_eq!(budget.snapshot().remaining_inferences, own().max_inferences);
}

#[test]
fn spending_after_a_window_ended_draws_on_the_next_one() {
    let time = Elapsed::none();
    let mut budget = self_managed(&time);
    spend_every_inference(&mut budget);
    time.pass(own().ttl_secs);

    budget.consume_inference(250, 0.0);

    assert_eq!(budget.remaining_inferences, own().max_inferences - 1);
    assert_eq!(budget.remaining_tokens, own().max_tokens - 250);
    let snapshot = budget.snapshot();
    assert!((snapshot.ttl_remaining_secs - own().ttl_secs as f64).abs() < f64::EPSILON);
}

/// Callers that read the counters rather than ask `has_remaining` need the
/// window started before they look.
#[test]
fn renewing_puts_the_next_window_in_the_counters() {
    let time = Elapsed::none();
    let mut budget = self_managed(&time);
    spend_every_inference(&mut budget);
    time.pass(own().ttl_secs);
    assert_eq!(budget.remaining_inferences, 0);

    assert!(budget.renew_if_expired());

    assert_eq!(budget.remaining_inferences, own().max_inferences);
    assert!(!budget.renew_if_expired());
}

#[test]
fn a_grant_holds_for_its_window_and_then_gives_way_to_the_agents_own() {
    let time = Elapsed::none();
    let mut budget = self_managed(&time);
    let grant = BudgetAllocation {
        max_inferences: 2,
        ttl_secs: 120,
        ..BudgetAllocation::default()
    };

    budget.refresh(&grant);
    budget.consume_inference(10, 0.0);
    budget.consume_inference(10, 0.0);
    time.pass(119);

    assert!(!budget.has_remaining());
    assert_eq!(budget.snapshot().remaining_inferences, 0);

    time.pass(1);

    assert!(budget.has_remaining());
    assert_eq!(budget.snapshot().remaining_inferences, own().max_inferences);
}

#[test]
fn a_grant_replaces_a_spent_window_at_once() {
    let time = Elapsed::none();
    let mut budget = self_managed(&time);
    spend_every_inference(&mut budget);
    let grant = BudgetAllocation {
        max_inferences: 6,
        ..BudgetAllocation::default()
    };

    budget.refresh(&grant);

    assert!(budget.has_remaining());
    assert_eq!(budget.snapshot().remaining_inferences, 6);
}

#[test]
fn an_agent_with_no_allocation_of_its_own_waits_for_a_grant() {
    let time = Elapsed::none();
    let mut budget = AgentComputeBudget::passive_on(time.clock());
    assert!(!budget.has_remaining());

    budget.refresh(&BudgetAllocation {
        ttl_secs: 60,
        ..BudgetAllocation::default()
    });
    assert!(budget.has_remaining());
    time.pass(60);

    assert!(!budget.has_remaining());
    assert!(!budget.renew_if_expired());
    assert_eq!(budget.snapshot().status, "passive");
}

/// Memory and disk in use are readings, not something a window hands out.
#[test]
fn what_is_in_use_carries_into_the_next_window() {
    let time = Elapsed::none();
    let mut budget = self_managed(&time);
    budget.update_memory_usage(512 * 1024 * 1024);
    budget.update_disk_usage(64 * 1024 * 1024);

    time.pass(own().ttl_secs);

    let snapshot = budget.snapshot();
    assert_eq!(snapshot.used_memory_bytes, 512 * 1024 * 1024);
    assert_eq!(snapshot.used_disk_bytes, 64 * 1024 * 1024);
    assert!(snapshot.has_remaining);
}

#[test]
fn an_agent_over_its_memory_limit_is_not_let_off_by_a_new_window() {
    let time = Elapsed::none();
    let mut budget = self_managed(&time);
    budget.update_memory_usage(own().max_memory_bytes + 1);

    time.pass(own().ttl_secs);

    assert!(!budget.has_remaining());
}

#[tokio::test]
async fn the_budget_an_agent_starts_with_is_one_it_manages_itself() {
    let time = Elapsed::none();
    let budget = new_shared_compute_budget_on(time.clock());

    time.pass(own().ttl_secs * 3);

    let snapshot = budget.read().await.snapshot();
    assert!(snapshot.has_remaining);
    assert_eq!(snapshot.remaining_inferences, own().max_inferences);
}
