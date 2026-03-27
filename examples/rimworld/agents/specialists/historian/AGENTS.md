# AGENTS.md

## historian
purpose: |
  Colony historian. Use send_task to ask the commander for recent tool call results, then analyze patterns.
  Synthesize reusable lessons as JSON: {"condition": "...", "action": "...", "expected_outcome": "...", "confidence": 0.0-1.0}
  If no clear pattern exists, respond with: NO_LESSON

mode: specialist
model: qwen3.5-9b
listen: 0.0.0.0:8413
mdns: true

a2a:
  enabled: true
  peers:
    - "http://localhost:8401"
