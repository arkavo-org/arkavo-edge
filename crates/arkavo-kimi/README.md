# arkavo-hrm

Hierarchical Reasoning Model (HRM) for bounded, multi-step agent task orchestration.

## Features

- **Strategic Task Decomposition**: High-level Conductor for breaking down complex goals into manageable sub-tasks.
- **Burst-Based Execution**: Bounded execution contracts (BurstContracts) with strict time and budget enforcement.
- **Persistent Task Store**: Reliable storage and recovery of task states to survive agent restarts or crashes.
- **Context Handover**: Intelligent strategy for passing context and reasoning states between execution bursts.
- **Loop Detection**: Automated detection of redundant reasoning cycles to prevent strategic thrashing.
- **Verification Loop**: Integrated verification of sub-task results before global state progression.
