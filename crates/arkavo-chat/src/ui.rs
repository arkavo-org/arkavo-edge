use std::io::{self, Write};

/// Display a thinking animation while waiting for processing
#[allow(dead_code)]
pub fn show_thinking_animation() {
    print!("Thinking");
    io::stdout().flush().unwrap();
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(300));
        print!(".");
        io::stdout().flush().unwrap();
    }
    println!();
}

/// Display a response from the LLM
pub fn display_response(response: &str) {
    println!("\n🤖 {}", response);
}

/// Display an error message
#[allow(dead_code)]
pub fn display_error(error: &str) {
    println!("❌ Error: {}", error);
}

/// Display welcome message for interactive mode
#[allow(dead_code)]
pub fn display_welcome_message() {
    println!("\n🤖 Qwen3-0.6B is ready (local privacy-first LLM)");
    println!("🔐 All processing happens on your device - no data is sent to external services");
    println!("💬 Type 'exit' or 'quit' to end the session");
    println!("❓ Try asking 'What can you help me with?' to learn more\n");
}

/// Get user input from stdin
#[allow(dead_code)]
pub fn get_user_input() -> io::Result<String> {
    print!("> ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    
    Ok(input.trim().to_string())
}