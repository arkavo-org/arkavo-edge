//! Re-export of CompiledCircuit from arkavo-torg-circuits

use super::features::PreflightFeature;

/// Type alias for CompiledCircuit specialized to PreflightFeature
pub(super) type CompiledCircuit = arkavo_torg_circuits::CompiledCircuit<PreflightFeature>;
