# LLM Integration Testing Requirements Document
**Project:** Arkavo Edge LLM Integration Testing
**Target Hardware:** Apple Silicon M4 Mac Mini (Standard/Pro)
**Context:** Edge-First, Local-Hybrid Architecture

## 1. Executive Summary
This document defines requirements for LLM integration testing, with a specific focus on the **M4 Mac Mini platform**. The M4's increased memory bandwidth (120GB/s+) and enhanced NPU (Neural Engine) provide a baseline for high-performance, local-first agentic workflows within the Arkavo Edge ecosystem.

## 2. Test Objectives

### 2.1 Performance Benchmarking (M4 Optimization)

#### 2.1.1 Response Time Metrics
**Local Models (M4 Mac Mini - Ministral 8B / Llama 3.1 8B):**
- **TTFT (Time To First Token):** < 150ms (Leveraging Metal/NPU acceleration).
- **Generation Speed:** 
    - **M4 Standard:** > 45 tokens/sec.
    - **M4 Pro:** > 65 tokens/sec.
- **P95 Latency:** < 1.5s for 512-token context.

**Long-Context Processing:**
- **Context Prefill (32k tokens):** < 5s processing time before first token.
- **Effective Throughput:** Maintain > 30 t/s even as context window fills.

#### 2.1.2 Memory & Thermal Efficiency
- **Memory Pressure:** Ensure zero swap usage during inference of 8B models on 16GB/24GB base models.
- **Thermal Stability:** Performance must not degrade by >10% after 30 minutes of continuous high-load testing (stressing the Mac Mini's compact thermal design).

## 3. Test Scope & Integration

### 3.1 M4-Specific Hardware Testing

#### 3.1.1 Metal & NPU Acceleration (`arkavo-llama-cpp`)
- **Objective:** Verify `llama-cpp` is correctly utilizing the M4 GPU and Neural Engine.
- **Test Cases:**
  - Verify `metal` backend initialization in logs.
  - Benchmark performance delta between CPU-only and Metal-accelerated inference.
  - Validate support for `Q4_K_M` and `Q8_0` quantizations.

#### 3.1.2 Unified Memory Management
- **Objective:** Optimize buffer sharing between `arkavo-edge` (Rust) and the GPU.
- **Test Cases:**
  - Monitor `arkavo-router` memory footprint during model swapping.
  - Validate that reloading a model from disk into Unified Memory takes < 1.5s.

### 3.2 System Integration
- **Context Ledger:** Test speed of vector embedding generation using local M4 NPU.
- **Tool Calling:** Verify low-latency tool execution (< 50ms overhead) when running alongside inference.

### 3.3 Safety & Moderation (New Features)

#### 3.3.1 PreflightModerator (`arkavo-router`)
**Context:** Implemented in `fa9fd7e2`. Blocks harmful inputs *before* inference.
- **Objective:** Verify zero-leakage of prohibited prompts to the LLM.
- **Test Cases:**
  - **Latency Overhead:** Ensure moderation check adds < 5ms to total request time on M4.
  - **Pattern Matching:** Validate regex/keyword blocking for locally defined sensitive terms.
  - **Fail-Safe:** If moderator panics/errors, the request **MUST** be rejected (Closed Circuit).
  - **Bypass Attempts:** Test with base64, leet-speak, and injection attempts against the router.

#### 3.3.2 CircuitCheck Integration (`arkavo-critic`)
**Context:** Implemented in `72e038ee`. Post-generation validation layers.
- **Objective:** Verify that output "circuits" break execution when quality/safety thresholds are violated.
- **Test Cases:**
  - **Hallucination Circuit:** Trigger a "circuit break" when model invents non-existent APIs (validate against `arkavo-mcp` schema).
  - **Policy Circuit:** Verify rejection of outputs containing PII or prohibited content types.
  - **Recovery:** Ensure the Orchestrator receives a structured `CircuitBreakError` and initiates a retry/correction loop.
  - **Performance:** Critic evaluation must complete within 50ms (P95) to maintain interactive feel.

#### 3.3.3 Safety Validation Test Suite

| Test Case | Input | Expected Result |
| :--- | :--- | :--- |
| **PII detection blocks SSN** | "My SSN is 123-45-6789" | `Block(policy: "block_pii")` |
| **PII detection blocks credit card** | "Card: 4111-1111-1111-1111" | `Block(policy: "block_pii")` |
| **Clean input passes** | "What is the weather?" | `Allow` |
| **SQL injection blocked** | "DROP TABLE users;" | `Block(policy: "block_injection")` |
| **Shell command blocked** | "Run sudo rm -rf /" | `Block(policy: "block_shell")` |
| **Multiple circuits — first violation wins** | Input matches policy 2 of 3 | `Block(policy: "policy_2")` |
| **No circuits registered — always allow** | Any input | `Allow` |
| **Empty input** | "" | `Allow` (or configurable) |
| **Long input blocked** | 100k characters | `Block(policy: "length_limit")` |

## 4. Test Environment Requirements

### 4.1 Hardware Targets
- **M4 Mac Mini (Base):** 10-core CPU/GPU, 16GB Unified Memory.
- **M4 Pro Mac Mini:** 12/14-core CPU, up to 20-core GPU, 64GB Unified Memory (Target for 30B+ parameter models).

### 4.2 Software Stack
- **macOS:** Sequoia 15.1+ (optimized for M4).
- **Toolchain:** Rust 1.82+, Xcode 16.1+.
- **Inference:** `arkavo-llama-cpp` with GGUF support.

## 5. Execution Strategy

### 5.1 Local Benchmarking Suite
Run the following command on target hardware:
```bash
ARKAVO_DEBUG=1 cargo run -p arkavo-bench -- --model ministral-8b --target-tps 45
```

### 5.2 Nightly Performance Regression
- Compare M4 results against M2/M3 baselines.
- Alert if M4 throughput drops below M3 Pro performance levels.

## 6. Success Criteria
- **M4 Efficiency:** Achieve > 50 tokens/sec on 8B models with < 15W package power.
- **Zero Swap:** 8B model inference must stay within physical memory boundaries.
- **Reliability:** 100% success rate on 1,000 consecutive tool-calling cycles.
