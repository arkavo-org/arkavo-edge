# Gemini Code Agent Example

<!-- ARKAVO-CAPABILITY: gemini -->
> **Specs**: [10 scenarios](../../specs/arkavo-edge/gemini.spec.yaml)
> **Browse**: `cargo xtask capabilities gemini`
<!-- /ARKAVO-CAPABILITY -->

This example demonstrates Arkavo's native integration with Google Gemini 2.5 Pro and Flash models for advanced coding tasks, with comprehensive benchmarking against Claude Code SDK.

## Overview

The Gemini Code Agent showcases:
- Google Gemini 2.5 Pro/Flash integration via `arkavo-gemini` crate
- 12 MCP tools for code analysis, security, and testing
- Streaming API for optimal performance (700-750ms TTFT)
- SWE-bench harness for objective quality evaluation
- Side-by-side performance comparison with Claude

## Performance Highlights

### Gemini 2.5 Flash
- **Speed**: 160 tokens/sec, 0.39s Time To First Token
- **Cost**: $0.30/M input, $2.50/M output
- **Context**: 1M tokens
- **Best for**: Fast iteration, development, testing

### Gemini 2.5 Pro
- **SWE-bench**: 63.8-67.2% verified score
- **Strengths**: #1 on WebDev Arena, front-end coding
- **Context**: 1M tokens
- **Best for**: Production code, complex projects

### Gemini 2.5 Flash-Lite
- **Speed**: 407-734 tokens/sec
- **Cost**: $0.10/M input, $0.40/M output
- **Best for**: Budget-conscious development

## Prerequisites

### 1. Install Arkavo

```bash
cd ../..
cargo build --release --features gemini
```

### 2. Get Gemini API Key

Visit [Google AI Studio](https://aistudio.google.com/app/apikey) to get your API key.

```bash
export GEMINI_API_KEY="your-api-key-here"
```

### 3. Install External Tools (Optional)

For full MCP tool functionality:

```bash
# Code search and analysis
brew install ripgrep
brew install comby
brew install tree-sitter

# Security scanning
brew install semgrep
pip install osv-scanner
brew install syft

# Testing (language-specific)
pip install pytest
npm install -g jest
```

## Quick Start

### 1. Start the Gemini Code Agent

```bash
./launch.sh
```

This starts an agent on port 8346 with Gemini 2.5 Pro.

For faster iteration with Flash:
```bash
./launch.sh --flash
```

### 2. Test Basic Functionality

```bash
# Simple code generation via chat
cargo run --features gemini -p arkavo -- chat --model gemini-flash-latest --prompt "Write a function to check if a number is prime"

# With streaming
cargo run --features gemini -p arkavo -- chat --model gemini-3-pro-preview --prompt "Create a React component for a todo list"
```

### 3. Run Benchmarks

```bash
# Run SWE-bench subset
./run_benchmarks.sh --model gemini-3-pro-preview --tasks 10

# Compare with Claude
./run_benchmarks.sh --compare claude-3-7-sonnet
```

### 4. Monitor Progress

```bash
# View agent logs
tail -f logs/gemini-code-agent.log

# Or use AGUI dashboard
arkavo ui
# Open http://localhost:3000
```

## Example Tasks

### 1. Frontend Component Generation (Gemini's Strength)

```bash
curl -X POST http://localhost:8346/v1/agent/task \
  -H "Content-Type: application/json" \
  -d '{
    "task": "Create a responsive React dashboard component with Tailwind CSS",
    "context": {
      "framework": "react",
      "styling": "tailwind",
      "components": ["header", "sidebar", "main-content", "footer"]
    }
  }'
```

### 2. Code Analysis with MCP Tools

```bash
curl -X POST http://localhost:8346/v1/agent/task \
  -H "Content-Type: application/json" \
  -d '{
    "task": "Analyze workspace/ for security vulnerabilities",
    "tools": ["sec_semgrep", "deps_osv"],
    "severity": "high"
  }'
```

### 3. Test Generation

```bash
curl -X POST http://localhost:8346/v1/agent/task \
  -H "Content-Type: application/json" \
  -d '{
    "task": "Generate comprehensive tests for TodoComponent.tsx",
    "framework": "jest",
    "coverage_target": 90
  }'
```

### 4. Code Refactoring

```bash
curl -X POST http://localhost:8346/v1/agent/task \
  -H "Content-Type: application/json" \
  -d '{
    "task": "Refactor workspace/api.ts to use async/await pattern",
    "tools": ["codegrep_search", "struct_find_replace"]
  }'
```

## Benchmarking

### SWE-bench Evaluation

The agent includes a SWE-bench harness for objective quality measurement:

```bash
# Run on SWE-bench Lite (300 instances)
./run_benchmarks.sh --dataset lite --count 50

# Run on SWE-bench Verified (500 instances)
./run_benchmarks.sh --dataset verified --count 20

# Custom GitHub issue
./run_benchmarks.sh --repo owner/repo --issue 123
```

### Performance Comparison

Compare Gemini against Claude on identical tasks:

```bash
# Generate comparison report
./compare_results.sh --output results/gemini_vs_claude.md

# Specific task comparison
./compare_results.sh --task frontend_component --iterations 5
```

### Metrics Tracked

- **Speed**: Time to first token, total completion time, tokens/sec
- **Cost**: Token usage, cost per task
- **Quality**: Code correctness, test passing rate, security issues
- **Tool Usage**: Tool calls, execution time, success rate

## MCP Tools Reference

### Code Search & Analysis
- **codegrep_search**: Fast repository-wide code search (ripgrep)
- **struct_find_replace**: Language-aware structural editing (Comby)
- **syntax_tree**: AST parsing for syntax analysis (tree-sitter)

### Security & Quality
- **sec_semgrep**: SAST scanning with OWASP/CWE rulesets
- **deps_osv**: Dependency vulnerability scanning
- **sbom_syft**: Software Bill of Materials generation

### Testing & Automation
- **test_run**: Multi-language test runner (pytest, jest, cargo test, go test)
- **browser_cdp**: Chrome DevTools Protocol automation

### GitHub Integration
- **gh_checks**: GitHub Checks API with inline annotations
- **gh_pr_review**: PR reviews with line-level comments

## Agent Configuration

The agent is configured via `code-agent-gemini.swarmkit.yaml`:

```yaml
roles:
  - id: "gemini-code-agent"
    role_type: "coding"
    mcp_tools:
      - server: "arkavo-code-search"
        tools: ["codegrep_search", "struct_find_replace", "syntax_tree"]
      - server: "arkavo-security"
        tools: ["sec_semgrep", "deps_osv"]
      - server: "arkavo-github"
        tools: ["test_run", "gh_checks", "gh_pr_review"]
runtime:
  listen: "0.0.0.0:8346"
```

The Gemini model itself (`gemini-3-pro-preview`, `gemini-flash-latest`, ...)
is a cloud model with no entry in this repo's local-edge-model vocabulary,
so it is not set via `agent_provisioning.model` in the kit — select it with
`GEMINI_MODEL` / `launch.sh`'s `--pro`/`--flash`/`--lite` flags instead (see
Model Selection below).

### Configuration not yet modeled in SwarmKit

The rest of the old AGENTS.md had fields with no RoleSpec equivalent,
supplied via env/flags/CLI defaults instead:
- `a2a.static_peers` — peer discovery is now mDNS-only (`runtime.mdns: true`)
- `websocket` — used only for `arkavo ui` connections, not agent-to-agent
- `logging`, `health_check` — CLI/runtime defaults
- `performance` (streaming, buffer_size, tool_timeout, parallel_tools, rate_limit)
- `workspace` (root, max_size_mb, auto_cleanup, cleanup_after_hours) — defaults to `./workspace`
- `code_generation.allow_globs`/`deny_globs` — no file-access-pattern equivalent yet

## Model Selection

### Gemini 2.5 Pro
Use for production-quality code generation:
```bash
export GEMINI_MODEL="gemini-3-pro-preview"
./launch.sh
```

### Gemini 2.5 Flash
Use for fast iteration and development:
```bash
export GEMINI_MODEL="gemini-flash-latest"
./launch.sh --flash
```

### Gemini 2.5 Flash-Lite
Use for budget-conscious development:
```bash
export GEMINI_MODEL="gemini-flash-lite-latest"
./launch.sh --lite
```

## Cost Optimization

### Budget Tracking

The agent automatically tracks token usage and costs:

```bash
# View current budget status
curl http://localhost:8346/v1/agent/budget

# Per-agent hourly/daily/monthly budget limits are not yet parsed from the
# kit (this was already true of the old AGENTS.md's commented-out `budget:`
# block); budget tracking is orchestrator-level. The kit's own
# `constraints.global_budget.max_cost_usd` and
# `agent_provisioning.budget.max_total_tokens` are enforced per session —
# see code-agent-gemini.swarmkit.yaml.
```

### Model Selection Strategy

- **Development**: Use Flash-Lite ($0.10/M input)
- **Testing**: Use Flash ($0.30/M input)
- **Production**: Use Pro (best quality/cost ratio)
- **Complex tasks**: Consider Claude for highest accuracy

## Troubleshooting

### API Key Issues

```bash
# Verify API key is set
echo $GEMINI_API_KEY

# Test API directly
curl "https://generativelanguage.googleapis.com/v1beta/models/gemini-3-pro-preview:generateContent?key=$GEMINI_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"contents":[{"parts":[{"text":"Hello"}]}]}'
```

### Feature Flag Missing

```bash
# Ensure gemini feature is enabled
cargo build --features gemini

# Check available features
cargo tree -p arkavo --features
```

### Performance Issues

```bash
# Enable streaming for better performance
export GEMINI_STREAMING=true

# Increase buffer size
export GEMINI_BUFFER_SIZE=16384

# Check logs for bottlenecks
grep "latency" logs/gemini-code-agent.log
```

### Tool Dependencies

```bash
# Check if required tools are installed
./launch.sh --check-deps

# Install missing tools
brew install ripgrep comby semgrep
```

## Gemini 3.0 Preparation

Gemini 3.0 is expected in Q1 2026 with:
- Multi-million token context window
- Built-in reasoning mode
- Enhanced multimodal capabilities

### Testing Gemini 3.0 Beta (October 9, 2025)

```bash
# When available, test with beta models
export GEMINI_MODEL="gemini-3.0-flash-beta"
./launch.sh

# Benchmark against 2.5 baseline
./run_benchmarks.sh --compare gemini-3-pro-preview
```

## Integration with Other Agents

The Gemini Code agent can work with other Arkavo agents:

```bash
# Start Claude Code agent for comparison
cd ../claude-code-agent
./launch.sh

# Start project orchestrator
cd ../software-development-lifecycle/orchestrator
./launch.sh

# Agents can delegate tasks to each other via A2A protocol
```

## Performance Tips

1. **Use Flash for iteration** - 2-5x faster, 60% cheaper than Pro
2. **Enable streaming** - Reduces TTFT by 30-50%
3. **Leverage MCP tools** - Use codegrep_search instead of full file reads
4. **Batch similar tasks** - Reduces API overhead
5. **Monitor token usage** - Track costs via budget API

## Benchmark Results

Results from testing on SWE-bench Lite subset (updated as testing progresses):

| Model | TTFT | Tokens/sec | Success Rate | Cost/Task |
|-------|------|------------|--------------|-----------|
| Gemini 2.5 Pro | TBD | TBD | TBD | TBD |
| Gemini 2.5 Flash | 0.39s | 160 | TBD | TBD |
| Claude 3.7 Sonnet | TBD | TBD | TBD | TBD |

Run `./run_benchmarks.sh --report` to generate updated results.

## Learn More

- [Gemini API Documentation](https://ai.google.dev/docs)
- [Arkavo Gemini Integration](../../crates/arkavo-gemini/README.md)
- [Coding Agent Toolset](../../docs/coding-agent-toolset.md)
- [SWE-bench Homepage](https://www.swebench.com/)
- [Arkavo Documentation](../../README.md)

## License

This example is part of the Arkavo project and follows the same license terms.
