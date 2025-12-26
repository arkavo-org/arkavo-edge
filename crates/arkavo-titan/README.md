# arkavo-titan

Low-latency runtime anomaly detection for TØRG policy graphs.

## Features

- **Surprise Detection**: Zero-copy inspection engine wrapping torg_core with sub-5µs overhead (p99).
- **Three-Level Monitoring**: Detection of hard failures, boundary violations, and statistical pattern drift.
- **EMA Tracking**: Lightweight Exponential Moving Average accumulator for low-overhead trend analysis.
- **Non-Blocking Feedback**: Dedicated "PainChannel" for reporting anomalies to self-healing auto-learners.
- **Boundary Detection**: Real-time identification of inputs falling outside the known-good policy hypercube.
- **Performance Optimized**: Designed for high-frequency evaluation loops where monitoring cost must be minimal.
