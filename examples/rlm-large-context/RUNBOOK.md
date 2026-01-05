# RLM Large Context Demo - Runbook

## What This Example Demonstrates

1. **RLM Activation**: Automatic context decomposition when input exceeds model limits
2. **Context Tools**: LLM using `context_search` and `context_probe` to navigate chunks
3. **Security Analysis**: Finding vulnerabilities in a large codebase with a small model

## Prerequisites

### Build Arkavo

```bash
cd /path/to/arkavo-edge
cargo build
```

### Verify Build

```bash
./target/debug/arkavo --version
# Expected: arkavo 0.52.0
```

## Step-by-Step Execution

### Step 1: Generate Synthetic Codebase

```bash
cd examples/rlm-large-context
chmod +x generate_codebase.sh
./generate_codebase.sh
```

**What to watch for:**
- Creates `synthetic_repo/` directory
- Generates ~25 Rust source files
- Reports ~100K+ tokens (25x an 8K context window)
- Lists 3 intentional vulnerabilities planted

### Step 2: Run the Analysis

```bash
chmod +x run_analysis.sh
./run_analysis.sh
```

**What to watch for:**

1. **RLM Activation Message**:
   ```
   [RLM] Context size: 102,400 tokens
   [RLM] Model context: 8,192 tokens
   [RLM] Activating RLM mode...
   ```

2. **Decomposition**:
   ```
   [RLM] Decomposing context into chunks...
   [RLM] Created manifest: rlm-XXXXXXXX
   [RLM] Chunks: 25
   ```

3. **Tool Calls** (if model supports tools):
   ```
   [TOOL] context_search("password", "auth", "sql")
   [TOOL] context_probe([3, 7, 15])
   ```

4. **Security Report**:
   - CRITICAL: MD5 password hashing in `auth/password.rs`
   - HIGH: SQL injection in `db/queries.rs`
   - MEDIUM: Missing rate limiting in `api/auth.rs`

### Step 3: Verify Findings

Check that the model found these vulnerabilities:

| File | Line | Vulnerability | Severity |
|------|------|--------------|----------|
| `auth/password.rs` | 12 | MD5 hashing | CRITICAL |
| `db/queries.rs` | 15 | SQL injection | HIGH |
| `db/queries.rs` | 45 | SQL injection in LIKE | HIGH |
| `api/auth.rs` | 25 | No rate limiting | MEDIUM |
| `auth/mod.rs` | 15 | Hardcoded secret | MEDIUM |

## Automated Validation

```bash
./run_analysis.sh 2>&1 | tee analysis.log

# Check for key outputs
grep -q "RLM" analysis.log && echo "✓ RLM activated"
grep -q "MD5\|password" analysis.log && echo "✓ Found password issue"
grep -q "SQL\|injection" analysis.log && echo "✓ Found SQL injection"
grep -q "rate\|limiting" analysis.log && echo "✓ Found rate limiting issue"
```

## Common Failure Modes

### RLM Not Activating

**Symptom**: No `[RLM]` messages in output

**Causes**:
1. Context not large enough (need >70% of model window)
2. Using cloud model with 128K context (try local model)

**Fix**:
```bash
# Force smaller context window detection
ARKAVO_MODEL_CONTEXT=8192 ./run_analysis.sh
```

### Model Not Using Tools

**Symptom**: No `context_search` or `context_probe` calls

**Causes**:
1. Model doesn't support tool use
2. RLM tools not registered

**Fix**: Use a tool-capable model (Claude, GPT-4, Gemini)

### Truncated Output

**Symptom**: Analysis cuts off mid-sentence

**Causes**:
1. Max tokens too low
2. Model timeout

**Fix**:
```bash
# Increase max tokens
ARKAVO_MAX_TOKENS=8192 ./run_analysis.sh
```

## Architecture Notes

### Why RLM for Security Analysis?

Traditional approach:
- Feed entire codebase to LLM
- Truncate to fit context window
- Miss 92% of the code

RLM approach:
- Decompose codebase into semantic chunks
- LLM searches for security-relevant code
- Probes only the chunks that matter
- Analyzes with full context of relevant code

### Chunk Strategy

The SemanticChunker in `arkavo-context` creates chunks:
- Target size: ~4KB (1024 tokens)
- Overlap: 200 bytes (preserves context at boundaries)
- Hints: Function names, keywords extracted for search

### Token Estimation

The `estimate_tokens()` function uses:
- 4 characters ≈ 1 token (rough approximation)
- More accurate than word-based estimates for code
- Sufficient for RLM activation decisions

## Cleanup

```bash
# Remove generated files
rm -rf synthetic_repo/
rm -f analysis.log
```
