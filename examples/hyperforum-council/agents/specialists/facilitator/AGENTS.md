# AGENTS.md

## facilitator
purpose: "Expert in productive discourse management. Identifies when discussion is stalling, going off-topic, or becoming unproductive. Suggests process improvements. Ensures all voices are heard. Manages turn-taking and time allocation. Detects and defuses conflict while preserving productive tension."
model: ministral-3b
listen: 0.0.0.0:8514
mdns: true
skills:
  - discussion_management
  - conflict_resolution
  - process_optimization
  - participation_balancing
  - topic_steering
  - productive_tension

a2a:
  enabled: true
  peers:
    - "http://localhost:8501"  # conductor
    - "http://localhost:8502"  # router
