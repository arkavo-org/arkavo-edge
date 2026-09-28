use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use serde_json::{Value, json};

use crate::{Executor, Library, Result, Routine, template};

#[derive(Default, Clone, Debug, Serialize)]
pub struct Metrics {
    pub observed_calls: u64,
    pub routine_attempts: u64,
    pub routine_successes: u64,
    pub primitive_calls: u64,
    pub failed_checks: u64,
    pub execution_micros: u64,
}

struct Observation {
    id: u64,
    tool: String,
    arguments: Value,
    result: Value,
    start: Instant,
    end: Instant,
}

#[derive(Default)]
struct State {
    observations: VecDeque<Observation>,
    metrics: Metrics,
}

/// Evidence belongs to one task; templates belong to an agent's shared library.
/// Raw observations never appear in library snapshots or the catalog.
pub struct Session {
    id: String,
    library: Arc<Mutex<Library>>,
    executor: Arc<dyn Executor>,
    state: Mutex<State>,
}

struct Attempt<'a> {
    library: &'a Mutex<Library>,
    id: &'a str,
    finished: bool,
}

impl Attempt<'_> {
    fn finish(mut self, success: bool) -> Result<()> {
        self.finished = true;
        self.library
            .lock()
            .map_err(|_| "Routine library lock poisoned")?
            .outcome(self.id, success)
    }
}

impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        if !self.finished
            && let Ok(mut library) = self.library.lock()
        {
            // Cancellation can leave external side effects with no verdict.
            // Retire this version rather than letting a retry duplicate them.
            let _ = library.interrupt(self.id);
        }
    }
}

impl Session {
    /// An unsuccessful or denied call breaks any candidate demonstration.
    pub fn invalidate_evidence(&self) -> Result<()> {
        self.state
            .lock()
            .map_err(|_| "Routine evidence lock poisoned")?
            .observations
            .clear();
        Ok(())
    }

    pub fn new(library: Arc<Mutex<Library>>, executor: Arc<dyn Executor>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            library,
            executor,
            state: Mutex::new(State::default()),
        }
    }

    pub fn observe(
        &self,
        tool: &str,
        arguments: Value,
        result: Value,
        start: Instant,
    ) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Routine evidence lock poisoned")?;
        state.metrics.observed_calls = state.metrics.observed_calls.saturating_add(1);
        let id = state.metrics.observed_calls;
        // Count oversized observations but don't retain them. The resulting
        // ID gap prevents admission of an apparently contiguous subsequence.
        if arguments.to_string().len() + result.to_string().len() > 65_536 {
            return Ok(());
        }
        state.observations.push_back(Observation {
            id,
            tool: tool.into(),
            arguments,
            result,
            start,
            end: Instant::now(),
        });
        if state.observations.len() > 32 {
            state.observations.pop_front();
        }
        drop(state);
        Ok(())
    }

    pub fn catalog(&self) -> Result<Value> {
        self.catalog_page(0, None)
    }

    pub fn catalog_page(&self, offset: usize, tool: Option<&str>) -> Result<Value> {
        if offset > 128 {
            return Err("Catalog offset exceeds library capacity".into());
        }
        let state = self
            .state
            .lock()
            .map_err(|_| "Routine evidence lock poisoned")?;
        let library = self
            .library
            .lock()
            .map_err(|_| "Routine library lock poisoned")?;
        let eligible: Vec<_> = library
            .catalog()
            .iter()
            .filter(|(_, r)| {
                r.routine
                    .steps
                    .iter()
                    .all(|s| self.executor.available(&s.tool))
                    && tool.is_none_or(|name| r.routine.steps.iter().any(|s| s.tool == name))
            })
            .collect();
        let value = json!({
            "routines": eligible.iter().skip(offset).take(4)
                .map(|(id,r)| json!({"id":id,"routine":r.routine,"active":r.active(),"retired":r.retired,"confidence":r.confidence()})).collect::<Vec<_>>(),
            "next_offset": (offset + 4 < eligible.len()).then_some(offset + 4),
            "observations": state.observations.iter().map(|o| json!({"id":o.id,"tool":o.tool})).collect::<Vec<_>>(),
            "metrics": state.metrics,
        });
        drop(eligible);
        drop(library);
        drop(state);
        Ok(value)
    }

    pub fn learn(&self, routine: Routine, inputs: &Value, ids: &[u64]) -> Result<String> {
        routine.validate()?;
        if ids.len() != routine.steps.len()
            || ids.windows(2).any(|w| w[1] != w[0].saturating_add(1))
        {
            return Err("Evidence must be a contiguous sequence, one observation per step".into());
        }
        let state = self
            .state
            .lock()
            .map_err(|_| "Routine evidence lock poisoned")?;
        let mut previous_end = None;
        for (step, id) in routine.steps.iter().zip(ids) {
            let observed = state
                .observations
                .iter()
                .find(|o| o.id == *id)
                .ok_or("Observation unavailable in this task")?;
            if !self.executor.available(&step.tool)
                || observed.tool != step.tool
                || observed.arguments != template::bind(&step.arguments, inputs)?
                || !step.check.matches(&observed.result, inputs)?
                || previous_end.is_some_and(|end| observed.start < end)
            {
                return Err(
                    "Routine does not match successful sequential execution evidence".into(),
                );
            }
            previous_end = Some(observed.end);
        }
        drop(state);
        self.library
            .lock()
            .map_err(|_| "Routine library lock poisoned")?
            .admit(routine, &self.id)
    }

    pub async fn run(&self, id: &str, inputs: &Value) -> Result<Value> {
        let routine = self
            .library
            .lock()
            .map_err(|_| "Routine library lock poisoned")?
            .resolve(id)?;
        // Bind and check the entire tool set before any side effects. The
        // executor must also check dynamic policy immediately before each call.
        let mut calls = Vec::new();
        for step in &routine.steps {
            if !self.executor.available(&step.tool) {
                return Err("Routine tool is unavailable or not granted".into());
            }
            calls.push(template::bind(&step.arguments, inputs)?);
            template::bind(&step.check.equals, inputs)?;
        }
        self.library
            .lock()
            .map_err(|_| "Routine library lock poisoned")?
            .begin(id)?;
        let attempt = Attempt {
            library: &self.library,
            id,
            finished: false,
        };
        self.state
            .lock()
            .map_err(|_| "Routine evidence lock poisoned")?
            .metrics
            .routine_attempts += 1;
        let start = Instant::now();
        let outcome = self.execute(&routine, calls, inputs).await;
        attempt.finish(outcome.is_ok())?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Routine evidence lock poisoned")?;
        state.metrics.execution_micros = state
            .metrics
            .execution_micros
            .saturating_add(start.elapsed().as_micros().min(u128::from(u64::MAX)) as u64);
        if outcome.is_ok() {
            state.metrics.routine_successes += 1;
        }
        outcome
    }

    async fn execute(&self, routine: &Routine, calls: Vec<Value>, inputs: &Value) -> Result<Value> {
        let mut last = Value::Null;
        for (index, (step, args)) in routine.steps.iter().zip(calls).enumerate() {
            self.state
                .lock()
                .map_err(|_| "Routine evidence lock poisoned")?
                .metrics
                .primitive_calls += 1;
            last = self.executor.execute(&step.tool, args).await
                .map_err(|_| format!("Routine stopped at step {index}: execution refused or failed; inspect state before retrying"))?;
            if !step.check.matches(&last, inputs)? {
                self.state
                    .lock()
                    .map_err(|_| "Routine evidence lock poisoned")?
                    .metrics
                    .failed_checks += 1;
                return Err(format!(
                    "Routine stopped at step {index}: completion check failed; inspect state before retrying"
                ));
            }
        }
        Ok(json!({"success":true,"steps":routine.steps.len(),"result":last}))
    }
}
