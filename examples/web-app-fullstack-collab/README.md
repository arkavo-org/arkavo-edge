# Fullstack Web App Collaboration Demo

## Goal
- Showcase two Arkavo agents collaborating over the A2A protocol to deliver a production-grade web app
- Split ownership between backend and frontend domains while sharing a single requirements backlog
- Demonstrate automated monitoring that feeds failures back into the agent loop for rapid fixes

## Components
- `backend-agent`: Owns APIs, database migrations, and service reliability
- `frontend-agent`: Owns UX, accessibility, and API client integration
- `project/`: Shared workspace containing requirements, docs, and generated code
- `run_scenarios.sh`: Issues feature, bug-fix, and enhancement tasks using A2A messages
- `monitoring/monitor_project.sh`: Runs local checks and reports results to agents

## Prerequisites
- `cargo build` from repo root to produce `target/debug/arkavo`
- Local Rust toolchain and Node.js toolchain if you want monitoring to run checks
- Optional: `arkavo ui` dashboard for live visualization

## Start The Agents
```bash
cd examples/web-app-fullstack-collab
./launch_agents.sh start
```
- Backend listens on `http://127.0.0.1:8351`
- Frontend listens on `http://127.0.0.1:8352`
- Logs written to `examples/web-app-fullstack-collab/logs`

## Drive Collaboration
```bash
./run_scenarios.sh bootstrap   # Dispatch initial feature work
./run_scenarios.sh bugfix      # Assign known regression fixes
./run_scenarios.sh enhance     # Trigger enhancement backlog
./run_scenarios.sh status      # Request live status from each agent
./run_scenarios.sh verify      # Ask agents to confirm their own test runs
```
Each call emits JSON-RPC `message/send` or `agent_query` requests so the agents coordinate without human prompts.

## Monitor And Close The Loop
```bash
./monitoring/monitor_project.sh verify   # Runs local tests and reports pass/fail via A2A
./monitoring/monitor_project.sh status   # Collects narrative status updates from both agents
```
- Monitoring script skips checks gracefully when toolchains or workspaces are missing
- Failures trigger `monitor_report` messages that agents are expected to triage and resolve

## Shared Requirements
- `project/requirements.md` tracks core stories, bugs, and enhancements
- Backend and frontend subdirectories include strategy notes plus regression expectations
- Docs under `project/docs/` must be kept in sync as contracts evolve; agents announce changes through A2A

## Cleanup
```bash
./launch_agents.sh stop
```
Logs remain under `logs/` for later analysis.

## Extending The Demo
- Add new scenarios to `run_scenarios.sh` for smoke tests, load tests, or release coordination
- Introduce additional agents (e.g., QA) by copying existing structure and updating `static_peers`
- Integrate CI by calling the monitoring script from automation and routing failures back to agents
