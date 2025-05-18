use arkavo_llm::HfTokenizer;

#[test]
fn test_hf_tokenizer_roundtrip() -> anyhow::Result<()> {
    // Load tokenizer from HuggingFace tokenizer.json file
    // Try different paths since tests run from different working directory
    let possible_paths = [
        "../models/tokenizer.json",
        "models/tokenizer.json",
        "./crates/arkavo-llm/models/tokenizer.json",
    ];
    
    let mut tokenizer = None;
    for path in possible_paths {
        if let Ok(t) = HfTokenizer::new(path) {
            println!("Found tokenizer at path: {}", path);
            tokenizer = Some(t);
            break;
        }
    }
    
    let tokenizer = tokenizer.ok_or_else(|| anyhow::anyhow!("Failed to load tokenizer from any path"))?;
    
    // Test with various inputs
    let inputs = [
        "Hello, world!",
        "Explain how to resolve a merge conflict in git.",
        "<|im_start|>system\nYou are Qwen3, a helpful AI assistant.\n<|im_end|>",
        "<|im_start|>user\nExplain how to resolve a merge conflict in git.\n<|im_end|>",
        "<|im_start|>assistant\nTo resolve a merge conflict in Git, follow these steps:\n\n1. First, identify the conflicting files by running `git status`.\n2. Open each conflicting file and look for the conflict markers (`<<<<<<<`, `=======`, `>>>>>>>`). \n3. Edit the file to fix the conflict by choosing one version or combining them.\n4. Remove the conflict markers.\n5. Save the file.\n6. Run `git add <filename>` to mark the conflict as resolved.\n7. Continue with the merge using `git merge --continue` or create a commit.\n\nIt's always a good practice to test your code after resolving conflicts to make sure everything works correctly.\n<|im_end|>"
    ];
    
    for (i, input) in inputs.iter().enumerate() {
        println!("\n=== Test case {} ===", i + 1);
        println!("Original: {}", input);
        
        // Encode (convert string to token IDs)
        let encoded = tokenizer.encode(input)?;
        println!("Tokens: {:?}", encoded.iter().take(10).collect::<Vec<_>>());
        println!("Token count: {}", encoded.len());
        
        // Decode (convert token IDs back to string)
        let decoded = tokenizer.decode(&encoded)?;
        println!("Decoded: {}", decoded);
        
        // For test assert
        assert_eq!(input, &decoded, "Roundtrip failed for test case {}", i + 1);
    }
    
    Ok(())
}