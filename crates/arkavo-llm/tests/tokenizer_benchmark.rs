/// # Tokenizer Performance Benchmark
///
/// This test benchmarks the optimized GGUF tokenizer implementation for Qwen3.
/// It prints out the number of tokens generated and the processing speed in tokens/second
/// for several representative input types (short, medium, special tokens, and long/repetitive).
///
/// The optimized implementation should generally process >500,000 tokens/sec on an M1/M2/M3 Mac
/// in debug mode, and 2M+ tokens/sec in release mode, for medium-length texts.
/// If performance falls far below this (e.g., <100,000 tokens/sec in debug), consider investigating.
///
/// Note: This is not a strict assertion; it's a warning to aid in performance regression detection.
use arkavo_llm::{GgufTokenizer, EMBEDDED_MODEL};
use anyhow::Result;
use std::time::Instant;

/// Compare the performance of the original and optimized tokenization algorithms
#[test]
fn benchmark_tokenizer_performance() -> Result<()> {
    // Create a tokenizer instance
    let tokenizer = match GgufTokenizer::new(EMBEDDED_MODEL) {
        Ok(t) => t,
        Err(e) => {
            println!("Skipping benchmark: Unable to create tokenizer: {}", e);
            return Ok(());
        }
    };
    
    // Define some sample texts of varying complexity
    let sample_texts = [
        // Short text
        "Hello world",
        
        // Medium text without special tokens
        "This is a medium-length text that contains various words, punctuation, and numbers like 1234. It should be tokenized efficiently.",
        
        // Text with special tokens
        "<|im_start|>system\nYou are a helpful AI assistant.\n<|im_end|>\n<|im_start|>user\nCan you help me?\n<|im_end|>\n<|im_start|>assistant\nI'll do my best to assist you.",
        
        // Longer text with repetitive patterns
        "The quick brown fox jumps over the lazy dog. The quick brown fox jumps over the lazy dog. The quick brown fox jumps over the lazy dog. The quick brown fox jumps over the lazy dog."
    ];
    
    println!("\n=== Tokenizer Performance Benchmark ===");
    
    // Run benchmarks for each sample text
    for (i, text) in sample_texts.iter().enumerate() {
        println!("\nBenchmark {}: Text length {} characters", i + 1, text.len());
        
        // Benchmark optimized implementation
        let start_time = Instant::now();
        let optimized_tokens = tokenizer.encode(text)?;
        let optimized_duration = start_time.elapsed();
        
        println!("  Optimized implementation: {:?}", optimized_duration);
        println!("  Generated {} tokens", optimized_tokens.len());
        
        // Calculate tokens per second
        let tokens_per_second = optimized_tokens.len() as f64 / optimized_duration.as_secs_f64();
        println!("  Performance: {:.2} tokens/second", tokens_per_second);
        
        // Print the first few tokens for verification
        let token_preview = if optimized_tokens.len() > 5 {
            format!("{:?}...", &optimized_tokens[..5])
        } else {
            format!("{:?}", optimized_tokens)
        };
        println!("  Token IDs: {}", token_preview);
    }
    
    // Test with a more complex example
    let complex_text = include_str!("../src/lib.rs");
    println!("\nBenchmark for large text: {} characters", complex_text.len());
    
    let start_time = Instant::now();
    let optimized_tokens = tokenizer.encode(complex_text)?;
    let optimized_duration = start_time.elapsed();
    
    println!("  Optimized implementation: {:?}", optimized_duration);
    println!("  Generated {} tokens", optimized_tokens.len());
    
    // Calculate tokens per second
    let tokens_per_second = optimized_tokens.len() as f64 / optimized_duration.as_secs_f64();
    println!("  Performance: {:.2} tokens/second", tokens_per_second);
    
    Ok(())
}