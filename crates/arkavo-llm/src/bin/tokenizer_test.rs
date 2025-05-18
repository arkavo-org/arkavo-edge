use anyhow::Result;
use arkavo_llm::EmbeddedQwen3Tokenizer;

fn main() -> Result<()> {
    // Initialize the tokenizer
    let tokenizer = EmbeddedQwen3Tokenizer::new()?;
    
    // Test string with special tokens
    let input = "<|im_start|>assistant\nExplain how to resolve a merge conflict in git.<|im_end|>";
    
    // Encode the input
    let tokens = tokenizer.encode(input)?;
    
    // Print the tokens for comparison
    println!("First 10 tokens: {:?}", &tokens[..10.min(tokens.len())]);
    println!("All tokens: {:?}", tokens);
    
    // Decode the tokens back to a string
    let decoded = tokenizer.decode(&tokens)?;
    println!("Decoded from tokens: {}", decoded);
    
    // Print the token count for reference
    println!("Token count: {}", tokens.len());
    
    Ok(())
}