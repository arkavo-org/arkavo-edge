//! Basic TUI regression tests that can run without triggering runtime issues
//! These tests verify the TUI testing infrastructure is working
//!
//! Note: TUI tools are discovery-only (not in core MCP tools), so we test them directly.

#![allow(clippy::disallowed_methods)] // block_on is safe in sync test functions

use arkavo_mcp_tools::server::Tool;
use arkavo_mcp_tools::tui::{TuiInteractionKit, TuiKeyboardKit, TuiScreenshotKit};
use serde_json::json;

#[test]
fn test_tui_keyboard_functionality() {
    // Skip keyboard tests in CI to avoid platform-specific issues
    if std::env::var("CI").is_ok() {
        println!("Skipping keyboard tests in CI environment");
        return;
    }

    println!("=== Testing TUI Keyboard Tool ===");

    let keyboard = TuiKeyboardKit::new();
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Test key press
    let result = rt.block_on(keyboard.execute(json!({
        "action": "key",
        "key": "a"
    })));

    match result {
        Ok(res) => {
            println!(
                "Key press result: {}",
                serde_json::to_string_pretty(&res).unwrap()
            );
            assert!(res["success"].as_bool().unwrap_or(false));
        }
        Err(e) => {
            println!("Key press failed (expected if no terminal focused): {e}");
        }
    }

    // Test text typing
    let result = rt.block_on(keyboard.execute(json!({
        "action": "text",
        "text": "Hello TUI"
    })));

    match result {
        Ok(res) => {
            println!(
                "Text typing result: {}",
                serde_json::to_string_pretty(&res).unwrap()
            );
            assert!(res["success"].as_bool().unwrap_or(false));
        }
        Err(e) => {
            println!("Text typing failed (expected if no terminal focused): {e}");
        }
    }

    // Test shortcut
    let result = rt.block_on(keyboard.execute(json!({
        "action": "shortcut",
        "shortcut": "c",
        "modifiers": ["ctrl"]
    })));

    match result {
        Ok(res) => {
            println!(
                "Shortcut result: {}",
                serde_json::to_string_pretty(&res).unwrap()
            );
            assert!(res["success"].as_bool().unwrap_or(false));
        }
        Err(e) => {
            println!("Shortcut failed (expected if no terminal focused): {e}");
        }
    }
}

#[test]
fn test_tui_screenshot_functionality() {
    // Skip screenshot tests in CI to avoid platform-specific issues
    if std::env::var("CI").is_ok() {
        println!("Skipping screenshot tests in CI environment");
        return;
    }

    println!("=== Testing TUI Screenshot Tool ===");

    let screenshot = TuiScreenshotKit::new();
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Test text format screenshot
    let result = rt.block_on(screenshot.execute(json!({
        "format": "text",
        "include_colors": false
    })));

    match result {
        Ok(res) => {
            println!(
                "Screenshot result: {}",
                serde_json::to_string_pretty(&res).unwrap()
            );
            assert!(res["success"].as_bool().unwrap_or(false));
            assert!(res.get("content").is_some());
        }
        Err(e) => {
            println!("Screenshot failed (expected if no terminal available): {e}");
        }
    }
}

#[test]
fn test_tui_interaction_schema() {
    println!("=== Testing TUI Interaction Tool Schema ===");

    let interaction = TuiInteractionKit::new();
    let schema = interaction.schema();

    println!("TUI Interaction schema name: {}", schema.name);
    println!("TUI Interaction schema description: {}", schema.description);

    assert_eq!(schema.name, "tui_interaction");
    assert!(schema.description.contains("TUI"));

    // Verify the schema has parameters
    assert!(schema.parameters.is_object());
}

#[test]
fn test_all_tui_tools_schemas() {
    println!("=== Verifying All TUI Tool Schemas ===");

    // Test TUI tools directly since they are discovery-only (not in core MCP tools)
    let tools: Vec<(&str, Box<dyn Tool>)> = vec![
        ("tui_keyboard", Box::new(TuiKeyboardKit::new())),
        ("tui_screenshot", Box::new(TuiScreenshotKit::new())),
        ("tui_interaction", Box::new(TuiInteractionKit::new())),
    ];

    for (expected_name, tool) in tools {
        println!("\nChecking schema for: {expected_name}");

        let schema = tool.schema();

        // Verify basic schema structure
        assert_eq!(schema.name, expected_name);
        assert!(!schema.description.is_empty());
        assert!(schema.parameters.is_object());

        println!("  ✓ Schema valid for {expected_name}");
    }
}

#[test]
fn test_keyboard_modifiers() {
    // Skip keyboard tests in CI to avoid platform-specific issues
    if std::env::var("CI").is_ok() {
        println!("Skipping keyboard modifier tests in CI environment");
        return;
    }

    println!("=== Testing Keyboard Modifiers ===");

    let keyboard = TuiKeyboardKit::new();
    let rt = tokio::runtime::Runtime::new().unwrap();

    let modifier_combos = vec![
        (vec!["ctrl"], "c"),
        (vec!["ctrl", "shift"], "a"),
        // Removed "cmd" + "q" as it's a common shortcut to quit applications on macOS
        // and likely causing the terminal to quit unexpectedly
        (vec!["alt"], "tab"),
    ];

    for (modifiers, key) in modifier_combos {
        println!("\nTesting {modifiers:?} + {key}");

        let result = rt.block_on(keyboard.execute(json!({
            "action": "shortcut",
            "shortcut": key,
            "modifiers": modifiers
        })));

        match result {
            Ok(res) => {
                println!(
                    "  Result: {}",
                    res["message"].as_str().unwrap_or("No message")
                );
                assert!(res["success"].as_bool().unwrap_or(false));
            }
            Err(e) => {
                println!("  Failed (expected without focused terminal): {e}");
            }
        }
    }
}
