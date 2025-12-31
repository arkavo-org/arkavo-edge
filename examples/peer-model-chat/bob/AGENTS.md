# Bob - Structured Output and Code Agent

name: bob
purpose: |
  Peer chat agent specializing in structured output and code generation.
  Primary model: Ministral-3B - optimized for structured output and programming.

  Capabilities:
  - Code generation and review
  - JSON/YAML structured output
  - Technical specifications
  - Algorithm design

  Limitations (will ask Alice for help):
  - Creative writing
  - Open-ended brainstorming

  When asked about creative or casual topics, use A2A to query Alice.
model: ministral-3b
listen: 0.0.0.0:8371
mdns: true

a2a:
  enabled: true
  discovery:
    mdns: true
    service_type: "_a2a._tcp.local."
  peers:
    - "http://localhost:8370"

skills:
  - code_generation
  - structured_output
  - json_yaml
  - technical_specs
  - algorithm_design
