# arkavo-autolearn

Auto-learning loop for self-healing agents implementing Phase 4 of Symbolic Boundary Evolution (SBE).

## Overview

This crate implements the four-step auto-learning cycle:

1. **Pain Signal**: Anomalies from runtime monitoring or proactive boundary probing
2. **Synthesis**: Ministral-3B generates TØRG graph within Adaptive Layer constraints
3. **Immune Response**: Agent verifies via InvariantLayer (distrusts its own LLM)
4. **Swarm Propagation**: Verified patches broadcast via gossip; receivers verify independently

## Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│                       AutoLearner                                │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────────────┐ │
│  │ PainAggregator│→│MinistralSynth │→│   ImmuneVerifier       │ │
│  │  (signals)    │  │  (LLM)       │  │ (InvariantLayer+SAT) │ │
│  └─────────────┘  └──────────────┘  └────────────────────────┘ │
│                                              ↓                  │
│                          ┌───────────────────────────────────┐ │
│                          │   GossipNetworkBridge             │ │
│                          │  (Patchlet broadcast + voting)    │ │
│                          └───────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

## Requirements Implemented

- **AUTO-001**: AutoLearner orchestrator struct
- **AUTO-002**: Ministral-3B integration for patchlet synthesis
- **AUTO-003**: Anomaly → synthesis → verification → propagation loop
- **AUTO-004**: Gossip protocol for swarm-wide patchlet distribution
- **AUTO-005**: Zero-trust verification for remote patches

## Usage

```rust
use arkavo_autolearn::{AutoLearner, AutoLearnerBuilder, AutoLearnConfig};
use arkavo_ensemble::{PolicyEnsemble, ConstantCost, PolicyLayer};
use arkavo_sbe::InvariantLayer;
use std::sync::Arc;

let learner = AutoLearnerBuilder::new()
    .synthesizer(synthesizer)
    .invariant_layer(Arc::new(InvariantLayer::new()))
    .network(network)
    .ensemble(ensemble)
    .build()?;

// Run the learning loop
let cancel = CancellationToken::new();
learner.run(cancel).await?;
```

## Configuration

### AutoLearnConfig

```rust
AutoLearnConfig {
    probe_interval: Duration::from_secs(60),  // How often to probe boundaries
    synthesis_threshold: 0.5,                  // Min severity to trigger synthesis
    synthesis_timeout: Duration::from_secs(30), // Timeout for LLM synthesis
    max_concurrent_synthesis: 2,               // Max parallel synthesis operations
    dry_run: false,                            // If true, don't broadcast patches
}
```

### Dry-Run Mode

For safe observation before enabling full self-healing:

```rust
let config = AutoLearnConfig {
    dry_run: true,  // Synthesize and verify, but don't broadcast
    ..Default::default()
};
```

When `dry_run` is enabled:
- Patches are synthesized normally
- Verification is performed
- Patches are logged but NOT broadcast to the network
- Statistics track `patches_dry_run` separately from `patches_broadcast`

## Features

- `llm` - Enable Ministral-3B synthesis (requires arkavo-torg, arkavo-llama-cpp)

Without the `llm` feature, synthesis operations return an error. This allows building on systems without LLM support.

```toml
[dependencies]
arkavo-autolearn = { version = "0.49", features = ["llm"] }
```

## Security

This crate implements zero-trust verification (AUTO-005):

- **Local patches**: Verified before broadcast via `verify()`
- **Remote patches**: More thorough verification via `deep_verify()` (2x timeout, exhaustive invariant checking)
- **All messages**: Cryptographically signed with agent keypair
- **Invariant contracts**: Prevent policy bypass and privilege escalation
- **Timeout protection**: Verification operations have configurable timeouts to prevent DoS

### Verification Timeouts

- Default timeout: 500ms for `verify()`
- Deep verification: 2x timeout (1000ms) for `deep_verify()`
- Synthesis timeout: 30s for LLM generation

Timeouts fail-safe by rejecting patches that take too long to verify.

## Pain Signal Sources

```rust
pub enum PainSource {
    RuntimeAnomaly { ... },    // Unexpected output during evaluation
    BoundaryProbe(probe),      // Proactive boundary found vulnerability
    PolicyHole(hole),          // SAT solver found policy hole
    External { description },  // Control plane signal
}
```

## Testing

```bash
# Run tests without LLM
cargo test -p arkavo-autolearn

# Run tests with LLM integration
cargo test -p arkavo-autolearn --features llm
```

## License

Apache-2.0 OR MIT
