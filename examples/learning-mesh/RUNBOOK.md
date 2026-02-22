# Learning Mesh Runbook

Step-by-step guide to running the learning mesh and observing the feedback loop.

## Prerequisites

```bash
# Build from repo root
cargo build

# Verify binary
ls target/debug/arkavo
```

## Step 1: Start the Agent Mesh

```bash
cd examples/learning-mesh
./launch.sh
```

Expected output:
```
[OK]   Arkavo binary found
[AGENT] Starting orchestrator (port 8410)...
[AGENT] Starting code-analyzer (port 8412)...
[AGENT] Starting test-generator (port 8414)...
[AGENT] Starting security-auditor (port 8416)...
[INFO]  Waiting for agents to initialize and discover peers...
```

Verify agents are running:
```bash
./launch.sh status
```

All four agents should show `[OK]`.

## Step 2: Start the AG-UI Gateway

In a separate terminal:

```bash
cargo run -p arkavo -- ui 7700
```

Open http://localhost:7700 in a browser.

## Step 3: Open the Learning Monitor

In a third terminal, watch for the three learning signals:

```bash
tail -f logs/orchestrator.log | grep -E 'Lesson extracted|Injecting.*guidance|quality='
```

## Step 4: Submit Tasks

Use the AG-UI web interface to submit tasks from `tasks.json`. Start with the
security tasks to observe the learning loop:

**Task 1: Security review (triggers learning)**

Paste into the AG-UI chat:
```
Review this authentication function for security issues:

pub fn authenticate(username: &str, password: &str) -> bool {
    let query = format!("SELECT * FROM users WHERE name='{}' AND pass='{}'", username, password);
    db::execute(&query).is_ok()
}
```

**What to observe:**
- Watch which agent gets selected (check the Cortical Routing Map panel)
- If a non-security agent handles it, expect a low quality score
- Look for "Lesson extracted" in the monitor terminal

**Task 2: Another security review (learning applied)**

```
Audit this token generation for cryptographic issues:

use rand::Rng;

pub fn generate_session_token() -> String {
    let mut rng = rand::thread_rng();
    let token: u64 = rng.gen();
    format!("{:x}", token)
}
```

**What to observe:**
- Thompson Sampling should now favor the security-auditor
- Watch for "Injecting N chars of behavior guidance" in the monitor
- Quality score should be higher than Task 1

**Task 3: Code review (different category)**

```
Review this error handling pattern:

pub fn process_request(data: &[u8]) -> String {
    let parsed = serde_json::from_slice(data).unwrap();
    let result = db::query(parsed).unwrap();
    let output = transform(result).unwrap();
    serde_json::to_string(&output).unwrap()
}
```

**What to observe:**
- New category (code_review) starts with fresh Thompson Sampling weights
- Security lessons should NOT affect code review routing
- This validates per-category independence

## Step 5: Observe the Connectome Panel

In the AG-UI web interface, look at the **Cortical Routing Map** panel:

- Each agent/category pair shows a quality sparkline
- Thompson Sampling weights update after each task
- Exploration tasks are marked with a different color
- Categories with lessons show accumulated guidance count

## Step 6: Verify Quality Trends

After 3+ tasks in the same category, the **Learning Status** panel shows:
- Quality trend graphs per agent/category
- Lesson count
- Routing history with quality scores

## Cleanup

```bash
./stop.sh
```

## Troubleshooting

**Agents show "initializing" in status check:**
Wait 5 more seconds for mDNS discovery, then retry `./launch.sh status`.

**No "Lesson extracted" messages:**
The response quality might be above 0.5 (good enough). Try submitting a vague
task that will produce a poor response: "Check the code" (no code provided).

**Guidance not being injected:**
Check that lessons exist: look for `PolicyCache` entries in the orchestrator log.
Guidance only appears when the PolicyCache has lessons for the task category.

**Port already in use:**
```bash
lsof -i :8410 -i :8412 -i :8414 -i :8416
./stop.sh
```
