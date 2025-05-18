use crate::{ChatConfig, ChatMessage, ChatRole, format_messages};
use anyhow::Result;
use arkavo_llm::{Qwen3Client, Qwen3Config};
use std::env;
use std::io::{self, Write};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::Duration;

/// Main chat session handling both interactive and one-shot modes
pub struct ChatSession {
    /// Chat configuration
    config: ChatConfig,
    
    /// Chat history
    history: Vec<ChatMessage>,
    
    /// LLM client
    llm_client: Option<Qwen3Client>,
}

impl ChatSession {
    /// Create a new chat session
    pub fn new(config: ChatConfig) -> Self {
        Self {
            config,
            history: Vec::new(),
            llm_client: None,
        }
    }
    
    /// Initialize the chat session by setting up the LLM client
    pub async fn init(&mut self) -> Result<()> {
        println!("Initializing Qwen3-0.6B LLM...");

        let llm_config = Qwen3Config {
            temperature: 0.7,
            use_gpu: true, // Try to use GPU acceleration (Metal on macOS ARM) for better performance
            max_tokens: 1024,
            ..Qwen3Config::default()
        };

        // Create and initialize LLM client
        let mut client = Qwen3Client::new(llm_config);
        client.init().await?;
        
        // Set up system message
        self.history.push(ChatMessage::system(
            "You are a helpful assistant that provides direct and concise answers to questions. \
             For simple questions, respond with just the answer without explanations unless asked for more details. \
             You're designed to assist with coding, documentation, and technical questions."
        ));

        self.llm_client = Some(client);
        
        Ok(())
    }
    
    /// Run the session in one-shot mode with a single prompt
    pub async fn run_one_shot(&mut self, prompt: &str) -> Result<String> {
        // Make sure the client is initialized
        if self.llm_client.is_none() {
            self.init().await?;
        }
        
        // Add the user message to history
        self.history.push(ChatMessage::user(prompt));
        
        // Show processing indicator
        if self.config.show_thinking {
            print!("Thinking");
            io::stdout().flush()?;
            for _ in 0..3 {
                thread::sleep(Duration::from_millis(300));
                print!(".");
                io::stdout().flush()?;
            }
            println!();
        }
        
        // Generate response
        let formatted_prompt = format_messages(&self.history);
        let client = self.llm_client.as_ref().unwrap();
        let response = client.generate(&formatted_prompt).await?;
        
        // Add the response to history
        self.history.push(ChatMessage::assistant(&response));
        
        Ok(response)
    }
    
    /// Run the session in interactive mode
    pub async fn run_interactive(&mut self) -> Result<()> {
        // Make sure the client is initialized
        if self.llm_client.is_none() {
            self.init().await?;
        }
        
        println!("\n🤖 Qwen3-0.6B is ready (local privacy-first LLM)");
        println!("🔐 All processing happens on your device - no data is sent to external services");
        println!("🚀 GPU acceleration is {}abled (Metal on Apple Silicon)", 
                 if self.llm_client.as_ref().unwrap().is_using_gpu() { "en" } else { "dis" });
        println!("💬 Type 'exit' or 'quit' to end the session");
        println!("❓ Try asking 'What can you help me with?' to learn more\n");

        // Interactive chat loop
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
                println!("Exiting chat session. Thank you for using Arkavo Edge!");
                break;
            }

            // Add user message to history
            self.history.push(ChatMessage::user(input));
            
            // Show "thinking" animation
            let thinking_stop = Arc::new(AtomicBool::new(false));
            let thinking_stop_clone = thinking_stop.clone();
            
            let thinking_thread = if self.config.show_thinking {
                Some(thread::spawn(move || {
                    let thinking_chars = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                    let mut i = 0;
                    while !thinking_stop_clone.load(Ordering::Relaxed) {
                        print!("\r🧠 {}", thinking_chars[i]);
                        io::stdout().flush().unwrap();
                        i = (i + 1) % thinking_chars.len();
                        thread::sleep(Duration::from_millis(100));
                    }
                }))
            } else {
                None
            };

            // Process with LLM
            let formatted_prompt = format_messages(&self.history);
            let client = self.llm_client.as_ref().unwrap();
            
            eprintln!("DEBUG: Calling LLM generate");
            
            // Add timeout to detect hangs
            // Use our own Duration to avoid confusion
            use std::time::Duration as StdDuration;
            let result = match tokio::time::timeout(StdDuration::from_secs(30), client.generate(&formatted_prompt)).await {
                Ok(generate_result) => {
                    eprintln!("DEBUG: LLM generate returned");
                    generate_result
                },
                Err(_) => {
                    eprintln!("DEBUG: LLM generate timed out after 30 seconds");
                    Err(anyhow::anyhow!("Model inference timed out after 30 seconds"))
                }
            };
            
            // Stop thinking animation
            if let Some(thinking_thread) = thinking_thread {
                thinking_stop.store(true, Ordering::Relaxed);
                if thinking_thread.join().is_err() {
                    // Ignore any errors from the thinking thread
                }
                print!("\r                      \r"); // Clear the thinking animation
                io::stdout().flush()?;
            }

            // Process response
            match result {
                Ok(response) => {
                    // Add the response to history
                    self.history.push(ChatMessage::assistant(&response));
                    
                    // Trim history if needed
                    while self.history.len() > self.config.max_history + 1 {  // +1 for system message
                        if self.history[0].role == ChatRole::System {
                            if self.history.len() > self.config.max_history + 2 {
                                self.history.remove(1);
                            } else {
                                break;  // Don't remove more if we only have system + max_history
                            }
                        } else {
                            self.history.remove(0);
                        }
                    }
                    
                    println!("🤖 {}", response);
                }
                Err(e) => {
                    println!("❌ Error generating response: {}", e);
                    println!("Please try a different query or check if the model files are correctly installed.");
                }
            }
            
            println!(); // Add a line break for better readability
        }

        Ok(())
    }
    
    // No need for file cleanup with in-memory model
    
    /// Get the current directory path
    pub fn get_current_directory() -> String {
        match env::current_dir() {
            Ok(path) => path.display().to_string(),
            Err(_) => String::from("Unknown"),
        }
    }
}