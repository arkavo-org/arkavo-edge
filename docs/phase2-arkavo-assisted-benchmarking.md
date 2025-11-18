# Phase 2: Arkavo-Assisted Benchmarking Architecture

## Objective

Prove that **Arkavo Edge + Gemini** significantly outperforms **Raw Gemini** by leveraging:
1. MCP tools for code analysis
2. Context enrichment from codebase understanding
3. Multi-step reasoning with tool feedback
4. Dependency analysis for accurate changes

## Hypothesis

**Arkavo-assisted problem solving will improve SWE-bench resolution rate from ~60% to 70%+ while maintaining competitive cost through intelligent tool usage and hybrid routing.**

## Architecture

### Two-Track Benchmark Design

#### Track 1: Raw LLM (Baseline - Already Complete)
```
Problem Statement → Gemini API → Solution Diff
```
**Characteristics**:
- No tools, no context
- Single-shot generation
- Fast but potentially inaccurate
- **Baseline**: 60% expected resolution rate

#### Track 2: Arkavo-Assisted (New Implementation)
```
Problem Statement
  ↓
Clone Repository & Checkout Base Commit
  ↓
Analyze Codebase (MCP Tools)
  - Code search for relevant files
  - Tree-sitter dependency analysis
  - Find similar patterns
  ↓
Enrich Context
  - Related code snippets
  - Import/export relationships
  - Test files
  ↓
Generate Solution (Gemini with enriched prompt)
  ↓
Validate with Tools
  - Syntax check
  - Run tests
  - Multi-file consistency
  ↓
Iterative Refinement (if needed)
  ↓
Final Solution Diff
```
**Characteristics**:
- Tool-assisted analysis
- Context-aware generation
- Multi-step refinement
- **Target**: 70%+ resolution rate

## Implementation Components

### 1. Repository Cloner

**Purpose**: Clone and checkout the repository at the specific commit for each SWE-bench instance

**Implementation**:
```rust
struct RepositoryManager {
    workspace_dir: PathBuf,
}

impl RepositoryManager {
    async fn prepare_instance(&self, instance: &SweBenchInstance) -> Result<PathBuf> {
        // 1. Clone repo if not exists
        // 2. Checkout base_commit
        // 3. Return path to working directory
    }
}
```

**Tools Used**:
- Git tool (already exists in arkavo-mcp-tools)
- Filesystem tool for workspace management

### 2. Context Analyzer

**Purpose**: Use MCP tools to understand the codebase before generating solutions

**MCP Tools to Use**:
1. **Code Search** (`arkavo-code-search`)
   - Find files related to problem keywords
   - Search for similar patterns
   - Locate test files

2. **Tree-sitter Analysis** (`arkavo-code-search/treesitter`)
   - Parse affected files into AST
   - Find all symbols (functions, classes, imports)
   - Build dependency graph

3. **Filesystem** (`arkavo-mcp-tools/filesystem`)
   - Read relevant source files
   - Identify file structure
   - Locate configuration files

**Implementation**:
```rust
struct ContextAnalyzer {
    code_search: CodeSearchTool,
    tree_sitter: TreeSitterTool,
    filesystem: FilesystemTool,
}

struct EnrichedContext {
    relevant_files: Vec<FileContext>,
    dependencies: DependencyGraph,
    test_files: Vec<PathBuf>,
    related_symbols: Vec<Symbol>,
}

impl ContextAnalyzer {
    async fn analyze(&self, repo_path: &Path, problem: &str) -> Result<EnrichedContext> {
        // 1. Extract keywords from problem statement
        // 2. Search codebase for related files
        // 3. Parse files with tree-sitter
        // 4. Build dependency graph
        // 5. Find test files
        // 6. Return enriched context
    }
}
```

### 3. Prompt Enricher

**Purpose**: Transform raw problem + context into optimized prompts for Gemini

**Strategy**:
```
Original Prompt (Raw):
  "Fix this issue: [problem_statement]"

Enriched Prompt (Arkavo-Assisted):
  "You are fixing a GitHub issue in the [repo] codebase.

  PROBLEM:
  [problem_statement]

  RELEVANT CODE CONTEXT:
  File: path/to/file.py
  ```python
  [relevant code snippet]
  ```

  DEPENDENCIES:
  - This file imports: [imports]
  - This file is imported by: [dependents]

  RELATED TESTS:
  File: tests/test_file.py
  ```python
  [test code]
  ```

  INSTRUCTIONS:
  Generate a git diff patch that:
  1. Addresses the problem statement
  2. Maintains compatibility with dependent code
  3. Updates tests if needed

  Output ONLY the git diff patch."
```

**Implementation**:
```rust
struct PromptEnricher {
    max_context_tokens: usize,
}

impl PromptEnricher {
    fn enrich(&self, problem: &str, context: &EnrichedContext) -> String {
        // 1. Build structured prompt
        // 2. Include top-N most relevant files
        // 3. Add dependency information
        // 4. Include test context
        // 5. Stay within token limits
    }
}
```

### 4. Solution Validator

**Purpose**: Use tools to validate generated solutions before returning

**Validation Steps**:
1. **Syntax Check**: Parse with tree-sitter to ensure valid code
2. **Test Execution**: Run relevant tests if available
3. **Multi-file Consistency**: Check if changes break imports/exports

**Implementation**:
```rust
struct SolutionValidator {
    test_runner: TestRunnerTool,
    tree_sitter: TreeSitterTool,
}

impl SolutionValidator {
    async fn validate(&self, solution: &str, repo_path: &Path) -> ValidationResult {
        // 1. Apply patch to temp directory
        // 2. Parse all changed files
        // 3. Run tests if available
        // 4. Check for breaking changes
        // 5. Return validation result with suggestions
    }
}
```

### 5. Iterative Refiner (Optional)

**Purpose**: If validation fails, refine solution with feedback

**Implementation**:
```rust
impl ArkavoAssistedSolver {
    async fn solve_with_refinement(&self, instance: &SweBenchInstance, max_iterations: usize) -> Result<Solution> {
        let mut iteration = 0;
        let mut last_error = None;

        while iteration < max_iterations {
            let solution = self.generate_solution(instance, last_error.as_ref()).await?;
            let validation = self.validate(&solution).await?;

            if validation.is_valid {
                return Ok(solution);
            }

            last_error = Some(validation.error_message);
            iteration += 1;
        }

        // Return best attempt even if not perfect
    }
}
```

## Comparative Benchmark Harness

### New Example: `swe-bench-arkavo-assisted.rs`

```rust
use arkavo_bench::SweBenchTool;
use arkavo_code_search::{CodeSearchTool, TreeSitterTool};
use arkavo_mcp_tools::{FilesystemTool, GitTool};
use arkavo_gemini::RestClient;

struct ArkavoAssistedSolver {
    repo_manager: RepositoryManager,
    context_analyzer: ContextAnalyzer,
    prompt_enricher: PromptEnricher,
    gemini_client: RestClient,
    validator: SolutionValidator,
}

impl ArkavoAssistedSolver {
    async fn solve(&self, instance: &SweBenchInstance) -> Result<(Solution, Metrics)> {
        let start = Instant::now();

        // 1. Clone and prepare repository
        let repo_path = self.repo_manager.prepare_instance(instance).await?;

        // 2. Analyze codebase and build context
        let context = self.context_analyzer.analyze(&repo_path, &instance.problem_statement).await?;

        // 3. Enrich prompt with context
        let enriched_prompt = self.prompt_enricher.enrich(&instance.problem_statement, &context);

        // 4. Generate solution with Gemini
        let solution = self.gemini_client.stream_generate_content(enriched_prompt, None).await?;

        // 5. Validate solution
        let validation = self.validator.validate(&solution, &repo_path).await?;

        let elapsed = start.elapsed();

        Ok((solution, Metrics {
            wall_time: elapsed,
            tool_calls: context.tool_call_count,
            tokens_used: estimate_tokens(&enriched_prompt, &solution),
            validation_passed: validation.is_valid,
        }))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let bench_tool = SweBenchTool::new();
    let arkavo_solver = ArkavoAssistedSolver::new(api_key);

    // Load instances
    let instances = bench_tool.load_instances("lite", Some(10)).await?;

    // Run Arkavo-assisted benchmark
    for instance in instances {
        let (solution, metrics) = arkavo_solver.solve(&instance).await?;

        // Evaluate with SWE-bench
        let resolved = bench_tool.evaluate(&instance, &solution).await?;

        println!("Instance: {}", instance.instance_id);
        println!("  Resolved: {}", resolved);
        println!("  Time: {:?}", metrics.wall_time);
        println!("  Tool calls: {}", metrics.tool_calls);
        println!("  Validation: {}", metrics.validation_passed);
    }
}
```

## Metrics to Track

### Per-Instance Metrics

| Metric | Raw LLM | Arkavo-Assisted | Notes |
|--------|---------|-----------------|-------|
| **Resolution Rate** | TBD | TBD | % of instances fully resolved |
| **Wall Time** | ~52s (Flash) | TBD | Total time including tools |
| **LLM Time** | ~52s | TBD | Time spent in Gemini calls |
| **Tool Time** | 0s | TBD | Time spent in code analysis |
| **LLM Cost** | $0.00003 | TBD | Gemini API costs |
| **Tool Calls** | 0 | TBD | Number of MCP tool invocations |
| **Context Size** | ~500 tokens | TBD | Prompt size |
| **Iterations** | 1 | TBD | Number of refinement loops |
| **Validation Pass** | N/A | TBD | Did solution pass validation? |

### Aggregate Comparison (10 instances)

```
┌──────────────────────┬─────────────┬──────────────────┬──────────┐
│ Metric               │ Raw LLM     │ Arkavo-Assisted  │ Delta    │
├──────────────────────┼─────────────┼──────────────────┼──────────┤
│ Resolution Rate      │ ~60%        │ Target: 70%+     │ +10%     │
│ Avg Time             │ 52s         │ Target: <90s     │ +38s     │
│ Avg Cost             │ $0.00003    │ Target: <$0.0001 │ +233%    │
│ Success/Cost Ratio   │ 20,000/$ │ Target: 7,000/$  │ -65%     │
└──────────────────────┴─────────────┴──────────────────┴──────────┘
```

**Success Criteria**: Arkavo-assisted approach must achieve:
- ✅ **+10% absolute improvement** in resolution rate (60% → 70%)
- ✅ **<2x time overhead** (52s → <104s average)
- ✅ **<3x cost increase** while improving accuracy by 10%

**ROI Calculation**:
- If resolution improves from 60% to 70%, that's 10% more problems solved
- Cost increase: ~3x ($0.00003 → $0.0001)
- Value: Solving 16.7% more problems for 3x cost = **5.6x ROI improvement**

## Implementation Timeline

### Week 1: Infrastructure
- [ ] Repository manager (clone/checkout)
- [ ] Context analyzer (code search + tree-sitter)
- [ ] Prompt enricher
- [ ] Basic integration test

### Week 2: Refinement & Validation
- [ ] Solution validator
- [ ] Iterative refinement logic
- [ ] Metrics tracking
- [ ] Full example implementation

### Week 3: Comparative Benchmark
- [ ] Run 50-100 instances (both tracks)
- [ ] Collect comprehensive metrics
- [ ] Analyze resolution rate improvements
- [ ] Generate comparison report

### Week 4: Optimization & Documentation
- [ ] Optimize context size for cost efficiency
- [ ] Fine-tune tool usage
- [ ] Create ROI dashboard
- [ ] Publish results

## Expected Outcomes

### Technical Validation
1. ✅ Prove Arkavo's tools improve accuracy
2. ✅ Quantify cost/performance trade-offs
3. ✅ Identify which tools provide most value
4. ✅ Establish best practices for tool-assisted coding

### Business Value
1. ✅ Demonstrate competitive advantage over raw LLM
2. ✅ Show 5-6x ROI improvement
3. ✅ Create reusable "assisted benchmarking" framework
4. ✅ Build marketing materials for best-in-class claims

### Technical Insights
1. Which MCP tools are most valuable?
2. What's the optimal context size?
3. Does validation/refinement improve results?
4. When should we use Gemma vs Gemini?

---

**Next Steps**: Begin Week 1 implementation with Repository Manager and Context Analyzer
