use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;

pub fn generate_schemas(check: bool, config: bool, wire: bool) -> Result<()> {
    let root_dir = project_root()?;
    let schemas_dir = root_dir.join("schemas");

    // Create schemas directory structure
    fs::create_dir_all(schemas_dir.join("openrpc"))?;
    fs::create_dir_all(schemas_dir.join("config"))?;
    fs::create_dir_all(schemas_dir.join("wire/v1"))?;

    // Always generate OpenRPC schema
    generate_openrpc_schema(&schemas_dir, check)?;

    // Generate config schemas if requested
    if config {
        generate_config_schemas(&schemas_dir, check)?;
    }

    // Generate wire protocol schemas if requested
    if wire {
        generate_wire_schemas(&schemas_dir, check)?;
    }

    Ok(())
}

fn generate_openrpc_schema(schemas_dir: &Path, check: bool) -> Result<()> {
    println!("Generating OpenRPC schema...");

    // Run the export-schema binary
    let output = Command::new("cargo")
        .args(["run", "-p", "arkavo-protocol", "--bin", "export-schema"])
        .output()
        .context("Failed to run export-schema")?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to generate schema: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let schema_content = String::from_utf8(output.stdout)?;
    let schema_path = schemas_dir.join("openrpc/arkavo-protocol.json");

    if check {
        // Check mode: verify the generated schema matches the committed one
        if schema_path.exists() {
            let existing_content = fs::read_to_string(&schema_path)?;

            let mut existing_json: serde_json::Value = serde_json::from_str(&existing_content)?;
            let mut generated_json: serde_json::Value = serde_json::from_str(&schema_content)?;

            // Ignore version field for comparison
            if let (Some(existing_info), Some(generated_info)) = (
                existing_json
                    .get_mut("info")
                    .and_then(|i| i.as_object_mut()),
                generated_json
                    .get_mut("info")
                    .and_then(|i| i.as_object_mut()),
            ) {
                existing_info.remove("version");
                generated_info.remove("version");
            }

            if existing_json != generated_json {
                eprintln!("❌ Schema has drifted from code!");
                eprintln!("Run 'cargo xtask schema-gen' to regenerate");

                // Show diff if possible
                if let Ok(output) = Command::new("git")
                    .args([
                        "diff",
                        "--no-index",
                        "--",
                        "-",
                        schema_path.to_str().unwrap(),
                    ])
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .and_then(|mut child| {
                        use std::io::Write;
                        if let Some(mut stdin) = child.stdin.take() {
                            stdin.write_all(schema_content.as_bytes()).ok();
                        }
                        child.wait_with_output()
                    })
                {
                    println!("{}", String::from_utf8_lossy(&output.stdout));
                }

                anyhow::bail!("Schema validation failed");
            }
            println!("✅ OpenRPC schema is up to date");
        } else {
            println!("⚠️  No existing schema found at {schema_path:?}");
            println!("Run 'cargo xtask schema-gen' without --check to generate it");
        }
    } else {
        // Write mode: update the schema file
        let pretty_json: serde_json::Value = serde_json::from_str(&schema_content)?;
        let pretty_content = serde_json::to_string_pretty(&pretty_json)?;

        fs::write(&schema_path, pretty_content)?;
        println!("✅ Wrote OpenRPC schema to {schema_path:?}");

        // Run git add to stage the change
        Command::new("git")
            .args(["add", schema_path.to_str().unwrap()])
            .status()
            .ok();
    }

    Ok(())
}

fn generate_config_schemas(schemas_dir: &Path, check: bool) -> Result<()> {
    println!("Generating config schemas...");

    use schemars::{schema_for, JsonSchema};

    // Generate schema for ServerConfig
    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct ServerConfigSchema {
        enabled: bool,
        bind_address: String,
        port: u16,
        max_connections: usize,
        idle_timeout_seconds: u64,
        metrics_enabled: bool,
        rate_limit: RateLimitConfigSchema,
        task_store_path: Option<String>,
    }

    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct RateLimitConfigSchema {
        max_requests_per_second: u32,
        burst_size: u32,
        enabled: bool,
    }

    let server_schema = schema_for!(ServerConfigSchema);
    let server_schema_json = serde_json::to_string_pretty(&server_schema)?;
    let server_schema_path = schemas_dir.join("config/server_config.json");

    if !check {
        fs::write(&server_schema_path, server_schema_json)?;
        println!("✅ Wrote ServerConfig schema to {server_schema_path:?}");
    }

    Ok(())
}

fn generate_wire_schemas(schemas_dir: &Path, check: bool) -> Result<()> {
    println!("Generating wire protocol schemas...");

    use schemars::{schema_for, JsonSchema};
    use serde::{Deserialize, Serialize};

    // Define simplified wire protocol types for schema generation
    #[derive(JsonSchema, Serialize, Deserialize)]
    struct PromiseRequest {
        promise_id: String,
        request_type: String,
        payload: serde_json::Value,
    }

    #[derive(JsonSchema, Serialize, Deserialize)]
    struct PromiseResponse {
        promise_id: String,
        result: Option<serde_json::Value>,
        error: Option<String>,
    }

    #[derive(JsonSchema, Serialize, Deserialize)]
    struct TaskRequest {
        task_id: String,
        task_type: String,
        parameters: serde_json::Value,
    }

    #[derive(JsonSchema, Serialize, Deserialize)]
    struct TaskResponse {
        task_id: String,
        status: String,
        result: Option<serde_json::Value>,
        error: Option<String>,
    }

    // Generate schemas
    let promise_req_schema = schema_for!(PromiseRequest);
    let promise_resp_schema = schema_for!(PromiseResponse);
    let task_req_schema = schema_for!(TaskRequest);
    let task_resp_schema = schema_for!(TaskResponse);

    if !check {
        // Write schemas to files
        let wire_dir = schemas_dir.join("wire/v1");

        fs::write(
            wire_dir.join("promise_request.json"),
            serde_json::to_string_pretty(&promise_req_schema)?,
        )?;

        fs::write(
            wire_dir.join("promise_response.json"),
            serde_json::to_string_pretty(&promise_resp_schema)?,
        )?;

        fs::write(
            wire_dir.join("task_request.json"),
            serde_json::to_string_pretty(&task_req_schema)?,
        )?;

        fs::write(
            wire_dir.join("task_response.json"),
            serde_json::to_string_pretty(&task_resp_schema)?,
        )?;

        println!("✅ Wrote wire protocol schemas to {wire_dir:?}");
    }

    Ok(())
}

fn project_root() -> Result<std::path::PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("Failed to find project root")?;

    if !output.status.success() {
        anyhow::bail!("Failed to find project root");
    }

    let path = String::from_utf8(output.stdout)?;
    Ok(Path::new(path.trim()).to_owned())
}
