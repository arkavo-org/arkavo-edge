use crate::{Result, TestError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// AI Analysis Engine that connects to Claude API for intelligent code analysis
/// This is the brain behind intelligent test generation
pub struct AnalysisEngine {
    client: reqwest::Client,
    api_key: Option<String>,
    model: String,
}

impl AnalysisEngine {
    pub fn new() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY").ok();
        
        Ok(Self {
            client: reqwest::Client::new(),
            api_key,
            model: "claude-3-sonnet-20240229".to_string(),
        })
    }
    
    pub fn with_api_key(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: Some(api_key),
            model: "claude-3-sonnet-20240229".to_string(),
        }
    }

    /// Analyze code to understand domain model and find testable properties
    pub async fn analyze_code(&self, code_context: &CodeContext) -> Result<DomainAnalysis> {
        let prompt = self.build_analysis_prompt(code_context);
        
        // For now, return a mock analysis
        // In production, this would call Claude API
        if self.api_key.is_none() {
            return Ok(self.mock_analysis(code_context));
        }
        
        // Real Claude API call would go here
        let response = self.call_claude_api(&prompt).await?;
        self.parse_analysis_response(&response)
    }

    /// Discover properties and invariants that should hold
    pub async fn discover_properties(&self, domain: &DomainAnalysis) -> Result<Vec<Property>> {
        let prompt = format!(
            "Given this domain model analysis: {:?}\n\
             Identify properties and invariants that should always be true.\n\
             Focus on:\n\
             1. Data consistency rules\n\
             2. Business logic constraints\n\
             3. State transition rules\n\
             4. Security properties\n\
             Format as JSON array of properties.",
            domain
        );
        
        if self.api_key.is_none() {
            return Ok(self.mock_properties());
        }
        
        let response = self.call_claude_api(&prompt).await?;
        self.parse_properties_response(&response)
    }

    /// Generate test cases that explore edge cases and try to break invariants
    pub async fn generate_test_cases(
        &self, 
        property: &Property,
        count: usize
    ) -> Result<Vec<TestCase>> {
        let prompt = format!(
            "Generate {} test cases that try to violate this property: {}\n\
             Focus on edge cases, boundary conditions, and unusual scenarios.\n\
             Include both valid and invalid inputs.\n\
             Format as JSON array of test cases.",
            count, property.invariant
        );
        
        if self.api_key.is_none() {
            return Ok(self.mock_test_cases(property, count));
        }
        
        let response = self.call_claude_api(&prompt).await?;
        self.parse_test_cases_response(&response)
    }

    /// Analyze test failure to provide actionable insights
    pub async fn analyze_failure(
        &self,
        test_case: &TestCase,
        error: &str
    ) -> Result<BugAnalysis> {
        let prompt = format!(
            "Analyze this test failure:\n\
             Test case: {:?}\n\
             Error: {}\n\
             Provide:\n\
             1. Root cause analysis\n\
             2. Minimal reproduction steps\n\
             3. Suggested fix\n\
             4. Severity assessment",
            test_case, error
        );
        
        if self.api_key.is_none() {
            return Ok(self.mock_bug_analysis(test_case, error));
        }
        
        let response = self.call_claude_api(&prompt).await?;
        self.parse_bug_analysis_response(&response)
    }

    fn build_analysis_prompt(&self, context: &CodeContext) -> String {
        format!(
            "Analyze this code to understand the domain model:\n\n\
             File: {}\n\
             ```\n{}\n```\n\n\
             Identify:\n\
             1. Core entities and their relationships\n\
             2. Business rules and constraints\n\
             3. State machines and transitions\n\
             4. Critical operations that need testing\n\
             5. Potential edge cases and failure modes\n\n\
             Format your response as a structured JSON analysis.",
            context.file_path, context.code
        )
    }

    async fn call_claude_api(&self, prompt: &str) -> Result<String> {
        let api_key = self.api_key.as_ref()
            .ok_or_else(|| TestError::Ai("No API key provided".to_string()))?;
        
        let request_body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": [{
                "role": "user",
                "content": prompt
            }]
        });
        
        let response = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| TestError::Ai(format!("API request failed: {}", e)))?;
        
        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(TestError::Ai(format!("API error: {}", error_text)));
        }
        
        let response_json: serde_json::Value = response.json().await
            .map_err(|e| TestError::Ai(format!("Failed to parse response: {}", e)))?;
        
        response_json["content"][0]["text"]
            .as_str()
            .ok_or_else(|| TestError::Ai("Invalid response format".to_string()))
            .map(|s| s.to_string())
    }

    fn parse_analysis_response(&self, response: &str) -> Result<DomainAnalysis> {
        // Extract JSON from response and parse
        serde_json::from_str(response)
            .map_err(|e| TestError::Ai(format!("Failed to parse analysis: {}", e)))
    }

    fn parse_properties_response(&self, response: &str) -> Result<Vec<Property>> {
        serde_json::from_str(response)
            .map_err(|e| TestError::Ai(format!("Failed to parse properties: {}", e)))
    }

    fn parse_test_cases_response(&self, response: &str) -> Result<Vec<TestCase>> {
        serde_json::from_str(response)
            .map_err(|e| TestError::Ai(format!("Failed to parse test cases: {}", e)))
    }

    fn parse_bug_analysis_response(&self, response: &str) -> Result<BugAnalysis> {
        serde_json::from_str(response)
            .map_err(|e| TestError::Ai(format!("Failed to parse bug analysis: {}", e)))
    }

    // Mock implementations for testing without API key
    fn mock_analysis(&self, _context: &CodeContext) -> DomainAnalysis {
        DomainAnalysis {
            entities: vec![
                Entity {
                    name: "User".to_string(),
                    attributes: vec!["id".to_string(), "balance".to_string()],
                    relationships: HashMap::new(),
                },
                Entity {
                    name: "Payment".to_string(),
                    attributes: vec!["amount".to_string(), "status".to_string()],
                    relationships: HashMap::from([
                        ("user".to_string(), "User".to_string())
                    ]),
                },
            ],
            operations: vec![
                Operation {
                    name: "process_payment".to_string(),
                    inputs: vec!["user_id".to_string(), "amount".to_string()],
                    outputs: vec!["payment_id".to_string()],
                    preconditions: vec!["user.balance >= amount".to_string()],
                    postconditions: vec!["user.balance = old(user.balance) - amount".to_string()],
                },
            ],
            invariants: vec![
                "user.balance >= 0".to_string(),
                "payment.amount > 0".to_string(),
            ],
            edge_cases: vec![
                "Concurrent payment processing".to_string(),
                "Payment with zero amount".to_string(),
                "User with maximum balance".to_string(),
            ],
        }
    }

    fn mock_properties(&self) -> Vec<Property> {
        vec![
            Property {
                name: "No negative balance".to_string(),
                description: "User balance should never go negative".to_string(),
                invariant: "forall user: user.balance >= 0".to_string(),
                category: PropertyCategory::DataIntegrity,
                severity: Severity::Critical,
            },
            Property {
                name: "Payment idempotency".to_string(),
                description: "Same payment request should not be processed twice".to_string(),
                invariant: "unique(payment.request_id)".to_string(),
                category: PropertyCategory::BusinessLogic,
                severity: Severity::High,
            },
        ]
    }

    fn mock_test_cases(&self, property: &Property, count: usize) -> Vec<TestCase> {
        (0..count).map(|i| TestCase {
            id: format!("test_{}", i),
            property: property.name.clone(),
            description: format!("Test case {} for {}", i, property.name),
            inputs: self.generate_mock_inputs(i),
            expected_behavior: if i % 5 == 0 {
                ExpectedBehavior::Failure("Constraint violation".to_string())
            } else {
                ExpectedBehavior::Success
            },
        }).collect()
    }

    fn generate_mock_inputs(&self, seed: usize) -> serde_json::Value {
        match seed % 5 {
            0 => serde_json::json!({ "amount": -50.0 }),  // Negative amount
            1 => serde_json::json!({ "amount": 1e10 }),   // Huge amount
            2 => serde_json::json!({ "amount": 0.001 }),  // Tiny amount
            3 => serde_json::json!({ "concurrent": true }), // Race condition
            _ => serde_json::json!({ "amount": 100.0 }),  // Normal case
        }
    }

    fn mock_bug_analysis(&self, test_case: &TestCase, _error: &str) -> BugAnalysis {
        BugAnalysis {
            test_case_id: test_case.id.clone(),
            root_cause: "Race condition in payment processing allows double charges".to_string(),
            minimal_reproduction: "1. Send two identical payment requests\n2. Both succeed".to_string(),
            suggested_fix: "Add distributed lock on payment request ID".to_string(),
            severity: Severity::Critical,
            affected_components: vec!["PaymentProcessor".to_string()],
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeContext {
    pub file_path: String,
    pub code: String,
    pub language: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DomainAnalysis {
    pub entities: Vec<Entity>,
    pub operations: Vec<Operation>,
    pub invariants: Vec<String>,
    pub edge_cases: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Entity {
    pub name: String,
    pub attributes: Vec<String>,
    pub relationships: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Operation {
    pub name: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub preconditions: Vec<String>,
    pub postconditions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Property {
    pub name: String,
    pub description: String,
    pub invariant: String,
    pub category: PropertyCategory,
    pub severity: Severity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyCategory {
    DataIntegrity,
    BusinessLogic,
    Security,
    Performance,
    Concurrency,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    pub id: String,
    pub property: String,
    pub description: String,
    pub inputs: serde_json::Value,
    pub expected_behavior: ExpectedBehavior,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedBehavior {
    Success,
    Failure(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BugAnalysis {
    pub test_case_id: String,
    pub root_cause: String,
    pub minimal_reproduction: String,
    pub suggested_fix: String,
    pub severity: Severity,
    pub affected_components: Vec<String>,
}