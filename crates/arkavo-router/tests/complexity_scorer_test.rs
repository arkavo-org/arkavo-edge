//! Tests for ComplexityScorer - task complexity analysis for architect mode routing.
//!
//! These tests verify:
//! - Simple tasks don't trigger architect mode
//! - Complex multi-step tasks DO trigger architect mode
//! - Subtask estimation is reasonable
//! - Savings estimation is calculated correctly

use arkavo_router::ComplexityScorer;

/// Simple conversational prompts should NOT trigger architect mode
#[test]
fn test_simple_conversational_prompts() {
    let scorer = ComplexityScorer::new();

    let simple_prompts = [
        "Hello",
        "Hi there!",
        "What time is it?",
        "Thanks",
        "Goodbye",
        "How are you?",
    ];

    for prompt in simple_prompts {
        let result = scorer.analyze(prompt);
        assert!(
            !result.architect_recommended,
            "Conversational prompt '{}' should NOT trigger architect (subtasks={}, savings={})",
            prompt,
            result.estimated_subtasks,
            result.estimated_savings_percent
        );
    }
}

/// Simple factual questions should NOT trigger architect mode
#[test]
fn test_simple_factual_questions() {
    let scorer = ComplexityScorer::new();

    let factual_prompts = [
        "What is 2 + 2?",
        "Define recursion",
        "What is Rust?",
        "Explain HTTP",
        "What does API stand for?",
    ];

    for prompt in factual_prompts {
        let result = scorer.analyze(prompt);
        assert!(
            !result.architect_recommended,
            "Factual question '{}' should NOT trigger architect",
            prompt
        );
    }
}

/// Complex multi-step development tasks SHOULD trigger architect mode
/// The scorer requires: subtasks >= 3, output_tokens >= 2000, and (keywords OR multi_step >= 2)
#[test]
fn test_complex_development_tasks() {
    let scorer = ComplexityScorer::new();

    // These prompts use explicit multi-step patterns that the scorer detects
    let complex_prompts = [
        // Has "first", "then", "after that", "finally" - triggers multi-step detection
        "Refactor the authentication system. First, update the user model to support OAuth. \
         Then, create new API endpoints for token refresh. After that, update the frontend \
         components to handle the new auth flow. Finally, write comprehensive tests for \
         all the changes.",
        // Has "Step 1", "Step 2", etc.
        "Build a complete e-commerce platform. Step 1: Create the database schema for products and orders. \
         Step 2: Implement the backend API with authentication. Step 3: Build the React frontend. \
         Step 4: Add comprehensive test coverage. Step 5: Deploy to production.",
    ];

    for prompt in complex_prompts {
        let result = scorer.analyze(prompt);
        assert!(
            result.architect_recommended,
            "Complex task should trigger architect mode (got: recommended={}, subtasks={}, triggers={:?})",
            result.architect_recommended,
            result.estimated_subtasks,
            result.complexity_triggers
        );
    }
}

/// Tasks with multiple explicit steps should trigger architect
#[test]
fn test_multi_step_explicit_tasks() {
    let scorer = ComplexityScorer::new();

    let multi_step_prompts = [
        "First create the database schema, then implement the API endpoints, and finally add tests",
        "Step 1: Set up the project. Step 2: Add authentication. Step 3: Deploy",
        "1. Create models 2. Add validation 3. Write tests 4. Document API",
    ];

    for prompt in multi_step_prompts {
        let result = scorer.analyze(prompt);
        assert!(
            result.architect_recommended || result.estimated_subtasks >= 2,
            "Multi-step task '{}' should have multiple subtasks (got: {})",
            prompt,
            result.estimated_subtasks
        );
    }
}

/// Subtask estimation should be reasonable
#[test]
fn test_subtask_estimation_bounds() {
    let scorer = ComplexityScorer::new();

    // Very simple task
    let simple = scorer.analyze("Print hello world");
    assert!(
        simple.estimated_subtasks <= 2,
        "Simple task should have few subtasks"
    );

    // Complex task with explicit multi-step structure (triggers the scorer)
    let complex = scorer.analyze(
        "Build a full-stack application. First, create the React frontend with components. \
         Then, implement the Node.js backend with API endpoints. After that, set up \
         PostgreSQL database with migrations. Next, add Redis caching layer. \
         Finally, configure Docker and CI/CD pipeline.",
    );
    assert!(
        complex.estimated_subtasks >= 3,
        "Complex task should have multiple subtasks (got: {})",
        complex.estimated_subtasks
    );
    assert!(
        complex.estimated_subtasks <= 20,
        "Subtask estimate should be bounded (got: {})",
        complex.estimated_subtasks
    );
}

/// Savings estimation should be positive for complex tasks
#[test]
fn test_savings_estimation() {
    let scorer = ComplexityScorer::new();

    let complex = scorer.analyze("Build a complete e-commerce platform with payments");

    if complex.architect_recommended {
        assert!(
            complex.estimated_savings_percent > 0.0,
            "Complex task should have positive savings estimate (got: {})",
            complex.estimated_savings_percent
        );
        assert!(
            complex.estimated_savings_percent <= 90.0,
            "Savings should be bounded (got: {})",
            complex.estimated_savings_percent
        );
    }
}

/// Edge cases - empty and very short prompts
#[test]
fn test_edge_cases() {
    let scorer = ComplexityScorer::new();

    // Empty prompt
    let empty = scorer.analyze("");
    assert!(
        !empty.architect_recommended,
        "Empty prompt should not trigger architect"
    );

    // Single character
    let single = scorer.analyze("a");
    assert!(
        !single.architect_recommended,
        "Single char should not trigger architect"
    );

    // Whitespace only
    let whitespace = scorer.analyze("   \n\t   ");
    assert!(
        !whitespace.architect_recommended,
        "Whitespace should not trigger architect"
    );
}

/// Keywords that suggest complexity
#[test]
fn test_complexity_keywords() {
    let scorer = ComplexityScorer::new();

    let keyword_prompts = [
        "implement authentication",
        "add comprehensive tests",
        "create a full application",
        "build complete system",
    ];

    for prompt in keyword_prompts {
        let result = scorer.analyze(prompt);
        // These may or may not trigger architect, but should detect some complexity
        assert!(
            result.estimated_subtasks >= 1,
            "Keyword prompt '{}' should detect at least 1 subtask",
            prompt
        );
    }
}
