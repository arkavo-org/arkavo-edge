# arkavo-critic

Verification pipeline for LLM response quality assurance and policy enforcement.

## Features

- **Multi-Stage Verification**: Pluggable pipeline for validating LLM outputs before final delivery.
- **Schema Validation**: Automated checks to ensure tool calls match their defined JSON schemas.
- **Policy Enforcement**: Runtime verification of Symbolic Boundary Evolution (SBE) invariants.
- **Semantic Coherence**: Local model-based validation of response logic and coherence.
- **Priority Execution**: Priority-ordered check execution to ensure low-latency feedback for simple validations.
- **Structured Evidence**: Detailed evidence collection and reporting for all verification steps.
