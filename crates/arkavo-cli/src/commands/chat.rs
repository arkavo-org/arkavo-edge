use std::env;
use std::io::{self, Write};

pub fn execute(_args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting chat session...");
    println!("Repository context: {}", get_current_directory());
    println!();
    
    // Simple chat loop
    loop {
        print!("> ");
        io::stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        
        if input == "exit" || input == "quit" {
            println!("Exiting chat session.");
            break;
        }
        
        // Echo for now - would connect to agent loop in full implementation
        println!("Agent: You said: {}", input);
    }
    
    Ok(())
}

fn get_current_directory() -> String {
    match env::current_dir() {
        Ok(path) => path.display().to_string(),
        Err(_) => String::from("Unknown")
    }
}