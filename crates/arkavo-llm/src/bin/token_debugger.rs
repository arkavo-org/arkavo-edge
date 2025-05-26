use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use anyhow::Result;

use arkavo_llm::{GgufTokenizer, EMBEDDED_MODEL, analyze_tokenization_debug, test_tokenization};

fn main() -> Result<()> {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    
    if args.len() < 2 {
        // No arguments provided, print usage and then run interactive mode
        print_usage();
        println!("\nEntering interactive mode...\n");
        return run_interactive_mode();
    }
    
    let command = &args[1];
    
    match command.as_str() {
        "analyze" => {
            if args.len() < 3 {
                eprintln!("Error: 'analyze' command requires input text or file path");
                print_usage();
                return Ok(());
            }
            
            let input = if args[2].starts_with("file:") {
                // Read from file
                let file_path = args[2].trim_start_matches("file:");
                fs::read_to_string(file_path)
                    .map_err(|e| anyhow::anyhow!("Failed to read file {}: {}", file_path, e))?
            } else {
                // Use the direct input
                args[2].clone()
            };
            
            // Load the tokenizer
            let tokenizer = GgufTokenizer::new(EMBEDDED_MODEL)?;
            
            // Run the analysis
            let analysis = analyze_tokenization_debug(&input, &tokenizer)?;
            println!("{}", analysis);
        },
        
        "test" => {
            let tokenizer = GgufTokenizer::new(EMBEDDED_MODEL)?;
            
            // Collect test inputs
            let inputs: Vec<String> = if args.len() > 2 {
                args[2..].to_vec()
            } else {
                // Default test inputs
                vec![
                    "Hello, world!".to_string(),
                    "<|im_start|>user\nTest message<|im_end|>".to_string(),
                    " ".to_string(),  // Just a space
                ]
            };
            
            // Convert to string slices
            let input_slices: Vec<&str> = inputs.iter().map(|s| s.as_str()).collect();
            
            // Run the test
            let results = test_tokenization(&tokenizer, &input_slices)?;
            println!("{}", results);
        },
        
        "interactive" => {
            run_interactive_mode()?;
        },
        
        _ => {
            eprintln!("Unknown command: {}", command);
            print_usage();
        }
    }
    
    Ok(())
}

fn print_usage() {
    println!("Token Debugger - Diagnose tokenization issues in GGUF models");
    println!("\nUSAGE:");
    println!("  token_debugger <command> [arguments]");
    println!("\nCOMMANDS:");
    println!("  analyze <text>           Analyze the tokenization of the text");
    println!("  analyze file:<path>      Analyze the tokenization of text from a file");
    println!("  test [inputs...]         Test tokenization with one or more inputs");
    println!("  interactive              Run in interactive mode");
    println!("\nEXAMPLES:");
    println!("  token_debugger analyze \"Hello, world!\"");
    println!("  token_debugger analyze file:input.txt");
    println!("  token_debugger test \"Hello\" \"World\" \"<|im_start|>user\"");
    println!("  token_debugger interactive");
}

fn run_interactive_mode() -> Result<()> {
    println!("Token Debugger Interactive Mode");
    println!("-------------------------------");
    println!("Enter text to analyze, or commands:");
    println!("  :help  - Show help");
    println!("  :quit  - Exit the program");
    println!("-------------------------------");
    
    // Load the tokenizer once
    println!("Loading tokenizer...");
    let tokenizer = GgufTokenizer::new(EMBEDDED_MODEL)?;
    println!("Tokenizer loaded with {} vocabulary entries", tokenizer.vocab_size());
    
    loop {
        print!("> ");
        io::stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();
        
        // Check for commands
        match input {
            ":help" => {
                println!("COMMANDS:");
                println!("  :help     - Show this help message");
                println!("  :quit     - Exit the program");
                println!("  :file <path> - Analyze text from a file");
                println!("  :test     - Run tests with default inputs");
                println!("\nAny other input will be analyzed for tokenization issues.");
            },
            ":quit" | ":exit" | ":q" => {
                println!("Goodbye!");
                break;
            },
            _ if input.starts_with(":file ") => {
                let file_path = input.trim_start_matches(":file ").trim();
                if Path::new(file_path).exists() {
                    match fs::read_to_string(file_path) {
                        Ok(content) => {
                            println!("Analyzing file: {}", file_path);
                            let analysis = analyze_tokenization_debug(&content, &tokenizer)?;
                            println!("{}", analysis);
                        },
                        Err(e) => {
                            eprintln!("Error reading file: {}", e);
                        }
                    }
                } else {
                    eprintln!("File not found: {}", file_path);
                }
            },
            ":test" => {
                // Run tests with default inputs
                let default_inputs = [
                    "Hello, world!",
                    "<|im_start|>system\nYou are an AI<|im_end|>",
                    "<|im_start|>user\nTest message<|im_end|>",
                    " ",  // Just a space
                    "TestwithnospacesandVERYlongwordstotesttokenizationofunusualtext",
                ];
                
                println!("Running tests with default inputs...");
                let results = test_tokenization(&tokenizer, &default_inputs)?;
                println!("{}", results);
            },
            _ => {
                // If not a command, analyze the input
                if !input.is_empty() {
                    let analysis = analyze_tokenization_debug(input, &tokenizer)?;
                    println!("{}", analysis);
                }
            }
        }
        
        println!();  // Add a blank line between iterations
    }
    
    Ok(())
}