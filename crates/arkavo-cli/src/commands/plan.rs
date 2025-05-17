use std::env;
use std::fs;
use std::path::Path;

pub fn execute(_args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    println!("Generating change plan...");
    println!("Repository: {}", get_current_directory());
    println!();
    
    let current_dir = env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    
    println!("Files in current directory:");
    match fs::read_dir(current_dir) {
        Ok(entries) => {
            let mut files = Vec::new();
            let mut dirs = Vec::new();
            
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                
                if path.is_dir() {
                    dirs.push(name);
                } else {
                    files.push(name);
                }
            }
            
            dirs.sort();
            files.sort();
            
            println!("Directories:");
            for dir in &dirs {
                println!("  {}/", dir);
            }
            
            println!("\nFiles:");
            for file in &files {
                println!("  {}", file);
            }
            
            println!("\nSummary:");
            println!("Found {} directories and {} files", dirs.len(), files.len());
            
            if !files.is_empty() {
                println!("\nPotential files to modify:");
                let source_files: Vec<&String> = files.iter()
                    .filter(|f| f.ends_with(".rs") || f.ends_with(".toml") || f.ends_with(".md"))
                    .collect();
                
                if !source_files.is_empty() {
                    for file in source_files {
                        println!("  {}", file);
                    }
                } else {
                    println!("  No source files found in current directory");
                }
            }
        },
        Err(e) => {
            eprintln!("Error reading directory: {}", e);
            return Err(e.into());
        }
    }
    
    Ok(())
}

fn get_current_directory() -> String {
    match env::current_dir() {
        Ok(path) => path.display().to_string(),
        Err(_) => String::from("Unknown")
    }
}