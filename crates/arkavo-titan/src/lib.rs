//! Titan Monitor - Low-Latency Runtime Anomaly Detection
//!
//! This crate implements Issue #420: a zero-copy inspection engine that wraps
//! `torg_core::evaluate()` to detect runtime anomalies with <5µs overhead (p99).
//!
//! # Architecture
//!
//! ```text
//! Transaction → TitanMonitor::evaluate() → SurpriseDetector → PainChannel
//!                   (<5µs overhead)         (classification)   (non-blocking)
//! ```
//!
//! # Three-Level Surprise Detection
//!
//! - **Level 1 (Hard Failure)**: Runtime panics, timeouts, or errors
//! - **Level 2 (Boundary Violation)**: Inputs outside "Known Good" hypercube
//! - **Level 3 (Statistical Drift)**: Anomalous pattern entropy increases
//!
//! # Example
//!
//! ```ignore
//! use arkavo_titan::{TitanMonitor, TitanConfig};
//! use std::sync::Arc;
//!
//! // Create monitor with default configuration
//! let (monitor, rx) = TitanMonitor::new(TitanConfig::default());
//!
//! // Evaluate with automatic anomaly detection
//! let result = monitor.evaluate(&graph, &inputs);
//!
//! // Check for anomalies (non-blocking)
//! while let Ok(evidence) = rx.try_recv() {
//!     println!("Anomaly detected: {:?}", evidence.level);
//! }
//! ```

mod accumulator;
mod detector;
mod evidence;
mod monitor;
pub mod sequence_anomaly;
pub mod sequence_bridge;

pub use accumulator::EmaAccumulator;
pub use detector::{BoundaryDetector, DriftDetector, HardFailureDetector, SurpriseDetector};
pub use evidence::{AnomalyDetails, AnomalyEvidence, AnomalyLevel};
pub use monitor::{TitanConfig, TitanMonitor};

// Re-export torg-core types for convenience
pub use torg_core::{EvalError, Graph};
