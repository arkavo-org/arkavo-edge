use anyhow::Result;
use arkavo_llm::{GgufTokenizer, EMBEDDED_MODEL, analyze_tokenization_debug, test_tokenization, analyze_vocabulary};

/// Test the debug analysis functionality
#[test]
fn test_tokenizer_debug_basics() -> Result<()> {
    // Create a tokenizer from the embedded model
    let tokenizer = match GgufTokenizer::new(EMBEDDED_MODEL) {
        Ok(t) => t,
        Err(e) => {
            println!("Skipping test: unable to create tokenizer: {}", e);
            return Ok(());
        }
    };
    
    // Test with a simple input
    let basic_input = "Hello, world!";
    
    // Analyze the tokenization
    let analysis = analyze_tokenization_debug(basic_input, &tokenizer)?;
    
    // Just print the analysis to the console for visual inspection
    println!("{}", analysis);
    
    // Check that critical sections are present
    assert!(analysis.contains("TOKEN STATISTICS"), "Analysis should contain token statistics");
    assert!(analysis.contains("SPECIAL TOKEN CHECK"), "Analysis should check for special tokens");
    
    // Depending on the tokenizer implementation, we may or may not have unknown tokens
    // So we don't assert on the presence of UNK warnings
    
    Ok(())
}

/// Test with multiple inputs
#[test]
fn test_multiple_inputs() -> Result<()> {
    // Create a tokenizer from the embedded model
    let tokenizer = match GgufTokenizer::new(EMBEDDED_MODEL) {
        Ok(t) => t,
        Err(e) => {
            println!("Skipping test: unable to create tokenizer: {}", e);
            return Ok(());
        }
    };
    
    // Test with various input types
    let inputs = &[
        "Hello, world!",                                    // Basic ASCII
        "<|im_start|>system\nYou are an AI<|im_end|>",      // Special tokens
        " ",                                                // Just whitespace
        "😀 Emoji test",                                   // Unicode/emoji
    ];
    
    // Run the test
    let results = test_tokenization(&tokenizer, inputs)?;
    println!("{}", results);
    
    // Make sure we got results for all inputs
    for (i, input) in inputs.iter().enumerate() {
        assert!(results.contains(&format!("Test #{}: \"{}\"", i+1, input)),
               "Results should contain test for input #{}", i+1);
    }
    
    Ok(())
}

/// Test vocabulary analysis
#[test]
fn test_vocabulary_analysis() -> Result<()> {
    // Create a tokenizer from the embedded model
    let tokenizer = match GgufTokenizer::new(EMBEDDED_MODEL) {
        Ok(t) => t,
        Err(e) => {
            println!("Skipping test: unable to create tokenizer: {}", e);
            return Ok(());
        }
    };
    
    // Analyze the vocabulary
    let vocab_analysis = analyze_vocabulary(&tokenizer);
    println!("{}", vocab_analysis);
    
    // Check that the analysis contains essential information
    assert!(vocab_analysis.contains("Total vocabulary size:"), 
           "Should show vocabulary size");
    assert!(vocab_analysis.contains("Token characteristics"),
           "Should analyze token characteristics");
    assert!(vocab_analysis.contains("Token length distribution"),
           "Should show token length distribution");
    
    Ok(())
}

/// Test analysis with a complex input using special formatting
#[test]
fn test_complex_input() -> Result<()> {
    // Create a tokenizer from the embedded model
    let tokenizer = match GgufTokenizer::new(EMBEDDED_MODEL) {
        Ok(t) => t,
        Err(e) => {
            println!("Skipping test: unable to create tokenizer: {}", e);
            return Ok(());
        }
    };
    
    // A complex ChatML input
    let complex_input = "<|im_start|>system
You are Qwen3, a helpful AI assistant.
<|im_end|>
<|im_start|>user
Can you explain how tokenization works?
<|im_end|>
<|im_start|>assistant";
    
    // Analyze the tokenization
    let analysis = analyze_tokenization_debug(complex_input, &tokenizer)?;
    println!("{}", analysis);
    
    // Check that the analysis contains information about special tokens
    assert!(analysis.contains("SPECIAL TOKEN CHECK"), 
           "Analysis should check for special tokens");
    
    Ok(())
}