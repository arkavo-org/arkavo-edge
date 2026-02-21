# Learning Mesh

A 4-agent mesh that learns from its own mistakes. When an agent produces a
low-quality response, the system extracts a lesson, injects corrective guidance
into future prompts, and re-routes tasks away from underperforming agents.

This demonstrates three capabilities working together:

- **Thompson Sampling** selects which agent handles each task, balancing
  exploration (trying new agents) with exploitation (using proven ones)
- **Response Quality Judging** scores every response (0.0-1.0) by detecting
  empty outputs, generic non-answers, hallucinated tools, and output loops
- **Lesson-Informed Prompting** extracts failure patterns from low-quality
  responses and prepends corrective guidance to subsequent prompts

## Architecture

```
                    ┌───────────────────┐
                    │   AG-UI Gateway   │
                    │   (port 7700)     │
                    │                   │
                    │  Thompson Sampler │
                    │  Response Judge   │
                    │  Lesson Extractor │
                    └────────┬──────────┘
                             │ routes tasks
              ┌──────────────┼──────────────┐
              │              │              │
     ┌────────▼─────┐ ┌─────▼──────┐ ┌────▼──────────┐
     │ Code Analyzer │ │ Test Gen   │ │ Security Audit │
     │ (port 8411)   │ │ (port 8412)│ │ (port 8413)   │
     └───────────────┘ └────────────┘ └───────────────┘
              ▲              ▲              ▲
              └──────────────┼──────────────┘
                             │ gossip lessons
                    ┌────────▼──────────┐
                    │   Orchestrator    │
                    │   (port 8410)     │
                    │                   │
                    │  PolicyCache      │
                    │  LearningBus      │
                    │  Behavior Guide   │
                    └───────────────────┘
```

## The Learning Loop

**Round 1** - No prior knowledge:
1. User submits "Review this auth code for security issues"
2. Thompson Sampling explores: routes to code-analyzer (random pick)
3. Code-analyzer responds with a generic "Looks fine"
4. Judge scores it 0.2 (generic non-answer for a security task)
5. Lesson extracted: "code-analyzer returns generic non-answers for security_scan"
6. Routing weight for code-analyzer on security tasks decreases

**Round 2** - Lesson applied:
1. User submits another security review task
2. Thompson Sampling exploits: routes to security-auditor (higher weight now)
3. Behavior guidance injected: "Avoid generic non-answers. Provide specific
   file paths, CVE references, and remediation code."
4. Security-auditor returns detailed vulnerability analysis
5. Judge scores it 0.85 (specific findings with code examples)
6. Routing weight for security-auditor on security tasks increases

After a few rounds, the mesh converges: security tasks go to the security
auditor, code quality tasks go to the code analyzer, and every agent receives
corrective guidance from lessons learned across the fleet.

## Quick Start

```bash
# Build (from repo root)
cargo build

# Start the mesh
cd examples/learning-mesh
./launch.sh

# Start the UI
cargo run -p arkavo -- ui 7700

# Watch the learning loop in real time
tail -f logs/orchestrator.log | grep -E 'Lesson extracted|Injecting.*guidance|quality='
```

## What to Watch For

Three log signals confirm the learning loop is working:

1. **Lesson extracted** (after a poor response):
   ```
   AG-UI: Lesson extracted for code-analyzer on security_scan: Agent code-analyzer returns generic non-answers
   ```

2. **Guidance injected** (on the next task):
   ```
   Injecting 542 chars of behavior guidance
   ```

3. **Quality improving** (over multiple rounds):
   ```
   AG-UI: Task abc123 quality=0.20, issues: ["Generic response"]
   AG-UI: Task def456 quality=0.75, issues: []
   AG-UI: Task ghi789 quality=0.85, issues: []
   ```

## Agents

| Agent | Port | Specialization |
|-------|------|----------------|
| orchestrator | 8410 | Task routing, lesson storage, behavior guidance |
| code-analyzer | 8412 | Code quality, complexity, anti-patterns |
| test-generator | 8414 | Unit tests, edge cases, coverage |
| security-auditor | 8416 | Vulnerabilities, OWASP, crypto misuse |

## Tasks

Pre-built tasks in `tasks.json` cover three categories:

- **security_scan** - SQL injection, weak crypto, auth bypass
- **code_review** - Error handling, complexity, dead code
- **test_generation** - Unit tests for data structures

## Stopping

```bash
./stop.sh
```

## How It Works (Internals)

The learning pipeline has two halves:

**Gateway side** (arkavo-agui):
- `response_judge::judge()` scores each response using heuristic rules
- `lesson_extractor::extract_lesson()` converts low scores into `Lesson` objects
- `gateway_routing::update_task_status()` sends lessons via `lesson_tx` channel

**Agent side** (arkavo-server):
- `LearningBus::start_lesson_receiver()` consumes lessons from the channel
- `PolicyCache::add_lesson()` stores them in a ring buffer (max 5 per agent/category)
- `conductor::execute_with_conductor_and_learning()` calls `get_behavior_guidance()`
  and prepends it to the LLM prompt before routing

Thompson Sampling lives in the gateway routing layer. Each agent/category pair
has a Beta distribution. Successes (quality > 0.5) add to alpha; failures add
to beta. The gateway samples from each distribution and picks the highest draw.
