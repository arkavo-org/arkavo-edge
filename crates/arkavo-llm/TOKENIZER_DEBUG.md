# Tokenizer Debugging Tools

This document describes the tokenizer debugging utilities available in Arkavo LLM to help diagnose tokenization issues.

## Overview

The tokenizer debugging toolkit provides several ways to diagnose tokenization problems:

1. A standalone CLI tool for interactive testing
2. Functions for detailed tokenization analysis
3. Utilities for comparing tokenization between different inputs or tokenizers

## Using the CLI Tool

The token debugger CLI provides a convenient way to test tokenization without modifying your code:

```bash
# Run in interactive mode
cargo run --bin token_debugger interactive

# Analyze a specific input
cargo run --bin token_debugger analyze "Hello, world!"

# Test multiple inputs
cargo run --bin token_debugger test "Hello" "<|im_start|>user" "Test with special chars: !@#$%"

# Analyze text from a file
cargo run --bin token_debugger analyze file:path/to/input.txt
```

## Using the Debug Functions

The library exports several functions for tokenization debugging:

```rust
use arkavo_llm::{GgufTokenizer, analyze_tokenization_debug, test_tokenization, compare_tokenization};

// Create a tokenizer
let tokenizer = GgufTokenizer::new(model_bytes)?;

// Detailed analysis of a single input
let analysis = analyze_tokenization_debug("Hello, world!", &tokenizer)?;
println!("{}", analysis);

// Quick test of multiple inputs
let test_results = test_tokenization(&tokenizer, &["Hello", "World", "<|im_start|>"])?;
println!("{}", test_results);

// Compare tokenization between similar inputs
let comparison = compare_tokenization(&tokenizer, &["Hello!", "Hello."])?;
println!("{}", comparison);
```

## Common Issues and Solutions

### High UNK Token Rate

If you see a high rate of unknown tokens (ID 0):

1. Check if your tokenizer vocabulary is complete
2. Ensure the tokenizer matches the model's expected vocabulary
3. Try an alternative tokenizer implementation (HF vs GGUF)

### Missing Special Tokens

If special tokens like `<|im_start|>` are missing:

1. Make sure your tokenizer has the correct special tokens for your model type
2. Check tokenizer.json for properly defined special tokens
3. Add missing special tokens to your vocabulary if needed

### Round-Trip Encoding/Decoding Issues

If input text doesn't round-trip correctly:

1. Check if your tokenizer's decode function properly handles special tokens
2. Verify the ID-to-token mapping is complete
3. Test with simpler inputs to isolate the issue

## Integrating Debugging into Your Workflow

To add debugging to your existing code:

```rust
// In your inference loop
let tokens = tokenizer.encode(prompt)?;

// Add debugging
if log_level == "debug" {
    let analysis = analyze_tokenization_debug(prompt, &tokenizer)?;
    println!("{}", analysis);
}

// Proceed with inference
let output = model.generate(&tokens, ...)?;
```

## Advanced Tokenizer Verification

For thorough verification of your tokenizer:

```rust
// Run through a standard test set
let test_set = [
    "Hello, world!",
    "<|im_start|>system\nYou are an assistant.<|im_end|>",
    "Tokens with special chars: !@#$%^&*()",
    "Code example: `let x = 5;`",
    "Unicode: 你好, お元気ですか? 😀👍",
    // Add more test cases relevant to your application
];

let results = test_tokenization(&tokenizer, &test_set)?;
println!("{}", results);
```

## Troubleshooting Guide

| Problem | Potential Cause | Solution |
|---------|-----------------|----------|
| High UNK rate | Incomplete vocabulary | Use HF tokenizer or fix vocabulary extraction |
| Lost content in decode | Incorrect token-to-string mapping | Fix reverse_vocab in tokenizer |
| Missing special tokens | Tokenizer/model mismatch | Ensure tokenizer and model are from same source |
| Whitespace handling issues | Wrong tokenizer type detection | Explicitly configure tokenizer type |

## References

- For GPT-2 style tokenizers: Should use 'Ġ' for spaces
- For SentencePiece tokenizers: Should use '▁' for spaces
- For ChatML models: Need `<|im_start|>`, `<|im_end|>`, etc.