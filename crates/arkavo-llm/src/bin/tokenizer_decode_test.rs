use arkavo_llm::EmbeddedQwen3Tokenizer;

fn main() {
    println!("Testing tokenizer decode for stray 'c' issue");
    
    // Create tokenizer
    let tokenizer = EmbeddedQwen3Tokenizer::new().expect("Failed to create tokenizer");
    
    // Test case: "Explain how to resolve a merge conflict in git"
    let test_text = "Explain how to resolve a merge conflict in git.";
    
    // First encode it
    let encoded = tokenizer.encode(test_text).expect("Failed to encode text");
    println!("Encoded token IDs: {:?}", encoded);
    
    // Then decode it back
    let decoded = tokenizer.decode(&encoded).expect("Failed to decode tokens");
    println!("Decoded text: {}", decoded);
    
    // Check if there's a stray 'c'
    if decoded.starts_with("c") {
        println!("ISSUE: Stray 'c' at beginning still exists");
    } else {
        println!("SUCCESS: No stray 'c' at beginning");
    }
    
    // Test with system message format
    let system_message = "<|im_start|>system\nYou are Qwen3, a helpful AI assistant.\n<|im_end|>";
    let encoded_system = tokenizer.encode(system_message).expect("Failed to encode system message");
    println!("\nSystem message encoded: {:?}", encoded_system);
    let decoded_system = tokenizer.decode(&encoded_system).expect("Failed to decode system tokens");
    println!("System message decoded: {}", decoded_system);
    
    // Test with user message format
    let user_message = "<|im_start|>user\nExplain how to resolve a merge conflict in git.\n<|im_end|>";
    let encoded_user = tokenizer.encode(user_message).expect("Failed to encode user message");
    println!("\nUser message encoded: {:?}", encoded_user);
    let decoded_user = tokenizer.decode(&encoded_user).expect("Failed to decode user tokens");
    println!("User message decoded: {}", decoded_user);
}