use std::fs;

fn main() {
    println!("💥 Generating Monorepo Chaos...");

    // 1. Create Workspace
    let _ = fs::remove_dir_all("demo_workspace"); // Clean up previous run
    fs::create_dir_all("demo_workspace").unwrap();
    fs::write("demo_workspace/Cargo.toml", r#"
[workspace]
members = ["core_lib", "service_a", "service_b", "service_c"]
resolver = "2"
"#).unwrap();

    // 2. Create Core Lib (The Source of Truth)
    create_crate("core_lib", r#"
    // BREAKING CHANGE: changed id from u32 to String
    pub fn process_id(id: String) { 
        println!("Processing {}", id); 
    }
    "#);

    // 3. Create Broken Services (The Noise)
    // Each service tries to pass an integer, causing massive type mismatch errors
    for service in ["service_a", "service_b", "service_c"] {
        create_crate_with_dep(service, r#"
        use core_lib::process_id;
        fn main() {
            // ERROR: Expected String, found integer
            // Repeated lines to simulate verbose logs
            process_id(100); 
            process_id(101);
            process_id(102);
            process_id(103);
            process_id(104);
            process_id(105);
            process_id(106);
            process_id(107);
            process_id(108);
            process_id(109);
"#, "core_lib");
    }

    println!("✅ Chaos Prepared. Run 'cargo check' in demo_workspace to see the horror.");
}

fn create_crate(name: &str, content: &str) {
    let path = format!("demo_workspace/{}", name);
    fs::create_dir_all(&path).unwrap();
    fs::create_dir_all(format!("{}/src", path)).unwrap();
    
    fs::write(format!("{}/Cargo.toml", path), format!(r#"
[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
"#, name)).unwrap();

    fs::write(format!("{}/src/lib.rs", path), content).unwrap();
}

fn create_crate_with_dep(name: &str, content: &str, dep: &str) {
    let path = format!("demo_workspace/{}", name);
    fs::create_dir_all(&path).unwrap();
    fs::create_dir_all(format!("{}/src", path)).unwrap();
    
    fs::write(format!("{}/Cargo.toml", path), format!(r#"
[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
{} = {{ path = "../{}" }}
"#, name, dep, dep)).unwrap();

    // Repeat lines to generate massive output
    let mut expanded_content = content.to_string();
    for i in 0..100 {
        expanded_content.push_str(&format!("            process_id({});\n", 200 + i));
    }
    expanded_content.push_str("        }
");

    fs::write(format!("{}/src/main.rs", path), expanded_content).unwrap();
}
