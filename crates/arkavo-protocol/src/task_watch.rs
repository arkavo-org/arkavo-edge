//! How long a remote task is polled (CLI-019).
//!
//! Polling lasts as long as what the task reports keeps changing.
//! Generating images can take far longer than an answer in text, and a
//! fixed timeout gave up on work that was still moving. `arkavo task`,
//! `arkavo chat` and the `arkavo ui` gateway all poll by this rule.

use std::time::{Duration, Instant};

use crate::types::{TaskGetResponse, TaskStatus};

/// A task that reports nothing new for this long is left running.
pub const QUIET_LIMIT: Duration = Duration::from_mins(5);

/// What a task last reported, and since when.
pub struct Watch {
    seen: Option<(TaskStatus, Option<String>, Option<u8>)>,
    changed_at: Instant,
    limit: Duration,
}

impl Default for Watch {
    fn default() -> Self {
        Self::with_limit(QUIET_LIMIT)
    }
}

impl Watch {
    pub fn with_limit(limit: Duration) -> Self {
        Self {
            seen: None,
            changed_at: Instant::now(),
            limit,
        }
    }

    /// Note what `task` reports now: its progress message when it changed,
    /// or `Err` once it has reported nothing new for the limit.
    pub fn observe(&mut self, task: &TaskGetResponse) -> Result<Option<String>, String> {
        let progress = task.progress.as_ref();
        let now = (
            task.status,
            progress.and_then(|p| p.message.clone()),
            progress.and_then(|p| p.percentage),
        );
        if self.seen.as_ref() == Some(&now) {
            if self.changed_at.elapsed() >= self.limit {
                return Err(still_running(&task.task_id, self.limit));
            }
            return Ok(None);
        }
        let message = now.1.clone();
        self.seen = Some(now);
        self.changed_at = Instant::now();
        Ok(message)
    }

    /// Note a poll of `task_id` that got no report, such as one the agent's
    /// rate limiter turned away: nothing new, so `Err` once the limit has
    /// passed. A task whose polls are all refused is not polled forever.
    pub fn unanswered(&self, task_id: &str) -> Result<(), String> {
        if self.changed_at.elapsed() >= self.limit {
            return Err(still_running(task_id, self.limit));
        }
        Ok(())
    }

    /// Note a poll of `task_id` answered with an error: `Ok` to poll again
    /// when the agent's rate limiter turned it away, `Err` with what to tell
    /// the user otherwise. The limiter is shared by every caller of the
    /// agent, so a burst from elsewhere can refuse a poll while the task
    /// runs on.
    pub fn refused(&self, task_id: &str, code: i32, message: &str) -> Result<(), String> {
        if crate::rate_limit::is_rate_limited(code, message) {
            return self.unanswered(task_id);
        }
        Err(format!("RPC error: {code} - {message}"))
    }
}

fn still_running(task_id: &str, limit: Duration) -> String {
    format!(
        "Task {task_id} is still running but has reported no change for {} minutes. \
         Read its result later with tasks/get and {{\"task_id\": \"{task_id}\"}}",
        limit.as_secs().div_ceil(60)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TaskProgress;
    use arkavo_test_macros::spec;

    fn working(message: Option<&str>) -> TaskGetResponse {
        TaskGetResponse {
            task_id: "t1".into(),
            status: TaskStatus::Working,
            result: None,
            error: None,
            progress: message.map(|m| TaskProgress {
                percentage: None,
                message: Some(m.into()),
                eta_seconds: None,
            }),
        }
    }

    /// CLI-019: polling goes on while progress changes and stops after the
    /// limit without a change, saying how to read the result.
    #[spec("CLI-019")]
    #[test]
    fn a_task_is_followed_while_it_changes() {
        let mut watch = Watch::with_limit(Duration::from_millis(50));
        assert_eq!(watch.observe(&working(None)), Ok(None));
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(
            watch.observe(&working(Some("step 1 of 8"))),
            Ok(Some("step 1 of 8".into()))
        );
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(
            watch.observe(&working(Some("step 2 of 8"))),
            Ok(Some("step 2 of 8".into()))
        );
        assert_eq!(watch.observe(&working(Some("step 2 of 8"))), Ok(None));
        std::thread::sleep(Duration::from_millis(60));
        let stopped = watch.observe(&working(Some("step 2 of 8"))).unwrap_err();
        assert!(stopped.contains("still running"), "{stopped}");
        assert!(stopped.contains("tasks/get"), "{stopped}");
    }

    /// CLI-019: a poll the agent did not answer is no change, so polls that
    /// are all turned away end at the same limit.
    #[spec("CLI-019")]
    #[test]
    fn an_unanswered_poll_is_no_change() {
        let mut watch = Watch::with_limit(Duration::from_millis(50));
        assert_eq!(
            watch.observe(&working(Some("step 1 of 8"))),
            Ok(Some("step 1 of 8".into()))
        );
        assert_eq!(watch.unanswered("t1"), Ok(()));
        std::thread::sleep(Duration::from_millis(60));
        let stopped = watch.unanswered("t1").unwrap_err();
        assert!(stopped.contains("Task t1 is still running"), "{stopped}");
    }

    /// CLI-019: only a refusal by the rate limiter is polled again; another
    /// error with the same code still ends the poll, and refusals end it
    /// once the task has been quiet for the limit.
    #[spec("CLI-019")]
    #[test]
    fn only_a_rate_limit_refusal_is_polled_again() {
        let refused = (-32001, "Rate limit exceeded");

        assert_eq!(Watch::default().refused("t1", refused.0, refused.1), Ok(()));
        assert_eq!(
            Watch::default().refused("t1", -32001, "Session not found"),
            Err("RPC error: -32001 - Session not found".into())
        );
        let quiet = Watch::with_limit(Duration::ZERO).refused("t1", refused.0, refused.1);
        assert!(quiet.unwrap_err().contains("still running"));
    }
}
