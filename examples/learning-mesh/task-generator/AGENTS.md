# AGENTS.md

## task-generator-agent
purpose: |
  Call send_task to send code review tasks to agents. Do not explain, just call tools.

  Step 1: call list_agents
  Step 2: call send_task with agent_id from step 1 and a Rust code snippet to review
  Step 3: repeat with a different agent and different code

  Example send_task args: {"agent_id": "security-auditor-agent", "task": "Review for SQL injection:\nlet q = format!(\"SELECT * WHERE id={}\", input);"}

model:   glm-4.7-flash
listen:  0.0.0.0:8418

discovery:
  mdns: true
