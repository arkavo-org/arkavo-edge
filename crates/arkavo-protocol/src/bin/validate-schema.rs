use arkavo_protocol::generate_openrpc_schema;
use std::process;

fn main() {
    // Generate the schema
    let schema = generate_openrpc_schema();

    // Validate structure
    let mut errors = Vec::new();

    // Check OpenRPC version
    if !schema.openrpc.starts_with("1.") {
        errors.push(format!("Invalid OpenRPC version: {}", schema.openrpc));
    }

    // Check required info fields
    if schema.info.title.is_empty() {
        errors.push("Missing info.title".to_string());
    }

    if schema.info.version.is_empty() {
        errors.push("Missing info.version".to_string());
    }

    // Check methods
    if schema.methods.is_empty() {
        errors.push("No methods defined".to_string());
    }

    // Validate each method
    for method in &schema.methods {
        if method.name.is_empty() {
            errors.push("Method has empty name".to_string());
        }

        // Check naming convention
        if !method
            .name
            .chars()
            .all(|c| c.is_lowercase() || c == '_' || c == '.')
        {
            errors.push(format!(
                "Method '{}' doesn't follow naming convention (lowercase with underscores)",
                method.name
            ));
        }

        // Warn about missing descriptions
        if method.description.is_none() {
            eprintln!("Warning: Method '{}' has no description", method.name);
        }

        // Check result
        if method.result.name.is_empty() {
            errors.push(format!("Method '{}' has no result name", method.name));
        }
    }

    // Check required methods exist
    let method_names: Vec<&str> = schema.methods.iter().map(|m| m.name.as_str()).collect();
    let required_methods = [
        "promise_request",
        "promise_declare",
        "agent_discover",
        "rpc.discover",
    ];

    for required in &required_methods {
        if !method_names.contains(required) {
            errors.push(format!("Missing required method: {required}"));
        }
    }

    // Report results
    if errors.is_empty() {
        println!("✅ Schema validation passed");
        process::exit(0);
    } else {
        eprintln!("❌ Schema validation failed:");
        for error in errors {
            eprintln!("  - {error}");
        }
        process::exit(1);
    }
}
