# AGENTS.md

## performance-optimizer-agent
purpose: Analyze and optimize performance bottlenecks including memory allocation, CPU usage, and async efficiency
model:   glm-4.7-flash
listen:  0.0.0.0:8404

# Performance Optimizer Agent
# Analysis areas:
# - Memory allocation patterns (avoid excessive heap)
# - CPU-bound operations (unnecessary copying)
# - I/O efficiency (batching, buffering)
# - Async overhead (unnecessary spawns)
# - Lock contention (mutex patterns)

# Optimization targets:
# - Router response <= 50ms
# - Binary size <= 60MB
# - Memory efficient for edge devices

# Tools:
# - Profile with cargo flamegraph
# - Benchmark with criterion
# - Memory analysis with heaptrack

discovery:
  mdns: true
