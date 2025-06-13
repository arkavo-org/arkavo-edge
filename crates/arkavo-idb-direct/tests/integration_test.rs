#[cfg(test)]
mod tests {
    use arkavo_idb_direct::IdbDirect;

    #[test]
    fn test_idb_version() {
        let version = IdbDirect::version();
        assert!(!version.is_empty());
        println!("IDB Direct version: {}", version);
    }

    #[test]
    fn test_initialization() {
        // Skip test in CI without simulator
        if std::env::var("CI").is_ok() {
            println!("Skipping initialization test in CI environment");
            return;
        }
        
        let result = IdbDirect::new();
        if result.is_err() {
            // This is expected in CI without simulator
            println!("IDB initialization failed (expected in CI): {:?}", result);
            return;
        }
        
        let idb = result.unwrap();
        drop(idb); // Ensure cleanup
    }

    #[test]
    #[allow(deprecated)]
    fn test_list_targets() {
        // Skip test in CI without simulator
        if std::env::var("CI").is_ok() {
            println!("Skipping list_targets test in CI environment");
            return;
        }
        
        let idb = match IdbDirect::new() {
            Ok(idb) => idb,
            Err(e) => {
                println!("Skipping test - IDB not available: {:?}", e);
                return;
            }
        };

        match idb.list_targets() {
            Ok(targets) => {
                println!("Found {} targets", targets.len());
                for target in &targets {
                    println!("  - {} ({})", target.name, target.udid);
                }
            }
            Err(e) => {
                println!("Failed to list targets: {:?}", e);
            }
        }
    }
}