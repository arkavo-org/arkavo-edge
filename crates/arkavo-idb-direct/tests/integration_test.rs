#[cfg(test)]
mod tests {
    use arkavo_idb_direct::{IdbDirect, TargetType};

    #[test]
    fn test_idb_version() {
        let version = IdbDirect::version();
        assert!(!version.is_empty());
        println!("IDB Direct version: {}", version);
    }

    #[test]
    fn test_initialization() {
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
    fn test_list_targets() {
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