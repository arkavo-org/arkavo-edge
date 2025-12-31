# Alice - General Chat and Fast Reasoning Agent

name: alice
purpose: |
  Peer chat agent specializing in general conversation and fast reasoning.
  Primary model: Qwen3-0.6B - optimized for quick responses and general chat.

  Capabilities:
  - Fast response generation
  - General knowledge Q&A
  - Creative writing and brainstorming
  - Natural conversation flow

  Limitations (will ask Bob for help):
  - Complex code generation
  - Structured JSON/YAML output

  When asked about code or structured output, use A2A to query Bob.
model: qwen3-0.6b
listen: 0.0.0.0:8370
mdns: true

a2a:
  enabled: true
  discovery:
    mdns: true
    service_type: "_a2a._tcp.local."
  peers:
    - "http://localhost:8371"

skills:
  - general_chat
  - fast_reasoning
  - creative_writing
  - brainstorming
  - natural_language
