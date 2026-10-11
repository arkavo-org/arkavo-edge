//! How long a remote task is polled: as long as what it reports keeps
//! changing (CLI-019). Generating images can take far longer than an answer
//! in text, and a fixed timeout gave up on work that was still moving.

use std::time::{Duration, Instant};

use arkavo_protocol::types::{TaskGetResponse, TaskStatus};

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
    use arkavo_protocol::types::TaskProgress;
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
}
