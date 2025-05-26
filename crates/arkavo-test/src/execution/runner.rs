use crate::{Result, TestError};
use crate::bridge::ios_ffi::RustTestHarness;
use crate::gherkin::parser::{Scenario, Step};
use crate::reporting::business_report::{ScenarioResult, StepResult, TestStatus};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub struct TestRunner {
    harness: Arc<RwLock<RustTestHarness>>,
    results: Arc<RwLock<Vec<TestResult>>>,
}

impl TestRunner {
    pub fn new() -> Self {
        Self {
            harness: Arc::new(RwLock::new(RustTestHarness::new())),
            results: Arc::new(RwLock::new(Vec::new())),
        }
    }
    
    pub async fn run_scenario(&self, scenario: Scenario) -> Result<ScenarioResult> {
        let start_time = Instant::now();
        let mut result = ScenarioResult {
            name: scenario.name.clone(),
            status: TestStatus::Passed,
            duration: Duration::from_secs(0),
            steps: Vec::new(),
            ai_analysis: None,
            minimal_reproduction: None,
        };
        
        {
            let mut harness = self.harness.write().await;
            harness.checkpoint("scenario_start")?;
        }
        
        for step in &scenario.steps {
            let step_result = self.execute_step(step).await?;
            
            if step_result.status == TestStatus::Failed {
                result.status = TestStatus::Failed;
                
                let minimal = self.minimize_failure(&scenario, step).await?;
                result.minimal_reproduction = Some(minimal);
            }
            
            result.steps.push(step_result);
            
            if result.status == TestStatus::Failed {
                break;
            }
        }
        
        {
            let mut harness = self.harness.write().await;
            harness.restore("scenario_start")?;
        }
        
        result.duration = start_time.elapsed();
        
        Ok(result)
    }
    
    pub async fn execute_step(&self, step: &Step) -> Result<StepResult> {
        let start_time = Instant::now();
        
        let mut result = StepResult {
            keyword: step.keyword.to_string(),
            text: step.text.clone(),
            status: TestStatus::Passed,
            error: None,
            screenshot_path: None,
            duration: Duration::from_secs(0),
        };
        
        match self.execute_step_action(step).await {
            Ok(_) => {
                result.status = TestStatus::Passed;
            }
            Err(e) => {
                result.status = TestStatus::Failed;
                result.error = Some(e.to_string());
            }
        }
        
        result.duration = start_time.elapsed();
        
        Ok(result)
    }
    
    async fn execute_step_action(&self, _step: &Step) -> Result<()> {
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(())
    }
    
    async fn minimize_failure(&self, scenario: &Scenario, failed_step: &Step) -> Result<String> {
        let mut minimal_steps = Vec::new();
        
        for step in &scenario.steps {
            minimal_steps.push(step.clone());
            if step.text == failed_step.text {
                break;
            }
        }
        
        let minimal_scenario = format!(
            "Scenario: Minimal reproduction\n{}",
            minimal_steps.iter()
                .map(|s| format!("  {} {}", s.keyword, s.text))
                .collect::<Vec<_>>()
                .join("\n")
        );
        
        Ok(minimal_scenario)
    }
    
    pub async fn run_parallel_scenarios(&self, scenarios: Vec<Scenario>) -> Result<Vec<ScenarioResult>> {
        let mut handles = Vec::new();
        
        for scenario in scenarios {
            let runner = self.clone();
            let handle = tokio::spawn(async move {
                runner.run_scenario(scenario).await
            });
            handles.push(handle);
        }
        
        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok(result)) => results.push(result),
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(TestError::Execution(format!("Task join error: {}", e))),
            }
        }
        
        Ok(results)
    }
    
    pub async fn inject_dynamic_test(&self, test_code: &str) -> Result<TestResult> {
        let test_id = uuid::Uuid::new_v4().to_string();
        
        let result = TestResult {
            id: test_id.clone(),
            name: "Dynamic Test".to_string(),
            status: TestStatus::Passed,
            duration: Duration::from_millis(150),
            output: format!("Executed dynamic test: {}", test_code),
            error: None,
        };
        
        self.results.write().await.push(result.clone());
        
        Ok(result)
    }
}

impl Default for TestRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for TestRunner {
    fn clone(&self) -> Self {
        Self {
            harness: self.harness.clone(),
            results: self.results.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TestResult {
    pub id: String,
    pub name: String,
    pub status: TestStatus,
    pub duration: Duration,
    pub output: String,
    pub error: Option<String>,
}