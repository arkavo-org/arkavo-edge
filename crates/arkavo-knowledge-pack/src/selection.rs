//! Choosing adapters for a session (KP-007).
//!
//! Selection is a decision about entitlement, so it happens over manifest
//! metadata and produces a set plus a ceiling — never a side effect. Loading
//! the chosen adapters is a separate step, and deliberately not implemented
//! yet: `llama_adapter_lora_init` takes a filesystem path and nothing else, so
//! a sealed adapter cannot reach llama.cpp without either an upstream change or
//! writing plaintext weights to disk. The second is what this whole design
//! exists to prevent, so the load half waits for the first.
//!
//! Compartments do not multiply adapters. Adapters partition by clearance
//! level; a compartment within a level is served by a capsule through the
//! existing per-role attribute release. An adapter per compartment would mean
//! stacking adapters that were never trained together.

use std::collections::BTreeSet;

use arkavo_gguf_tdf::Classification;

use crate::manifest::{ComponentRecord, PackManifest};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectionError {
    #[error(
        "selection spans classifications {levels:?}; the session must accept the {ceiling:?} \
         high-water ceiling before adapters from more than one level are stacked"
    )]
    MixedLevels {
        levels: Vec<Classification>,
        ceiling: Classification,
    },
}

/// What a session is entitled to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entitlements {
    /// Highest classification this session may see.
    pub clearance: Classification,
    /// Compartments the session holds. An adapter whose compartment is absent
    /// from this set is not selected however low its clearance.
    pub compartments: BTreeSet<String>,
    /// Whether the session has accepted that stacking adapters from several
    /// levels raises everything it produces to the highest of them.
    pub accepts_high_water: bool,
}

impl Entitlements {
    pub fn new(clearance: Classification) -> Self {
        Self {
            clearance,
            compartments: BTreeSet::new(),
            accepts_high_water: false,
        }
    }

    #[must_use]
    pub fn with_compartment(mut self, compartment: impl Into<String>) -> Self {
        self.compartments.insert(compartment.into());
        self
    }

    #[must_use]
    pub fn accepting_high_water(mut self) -> Self {
        self.accepts_high_water = true;
        self
    }
}

/// The adapters a session gets, and what their output carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Component file names, in manifest order.
    pub adapters: Vec<String>,
    /// The ceiling everything generated under this selection carries: the
    /// high-water mark of the selected adapters, or `Public` when none applies
    /// and the base model serves alone.
    pub ceiling: Classification,
}

impl Selection {
    /// Whether the base model serves this session unaided (KP-007 edge case).
    pub fn is_base_only(&self) -> bool {
        self.adapters.is_empty()
    }

    /// One line for the session's decision trace.
    pub fn trace(&self) -> String {
        if self.adapters.is_empty() {
            return "no adapter selected; base model serves".to_string();
        }
        format!(
            "adapters [{}] at ceiling {}",
            self.adapters.join(", "),
            self.ceiling.as_str()
        )
    }
}

/// Select the adapters a session is entitled to.
pub fn select_adapters(
    manifest: &PackManifest,
    entitlements: &Entitlements,
) -> Result<Selection, SelectionError> {
    let eligible: Vec<&ComponentRecord> = manifest
        .adapters()
        .filter(|adapter| is_entitled(adapter, entitlements))
        .collect();

    if eligible.is_empty() {
        // Not an error: a session entitled to no adapter is served by the base
        // model. Refusing here would deny service for lacking an optional part.
        return Ok(Selection {
            adapters: Vec::new(),
            ceiling: Classification::Public,
        });
    }

    let levels: BTreeSet<Classification> = eligible
        .iter()
        .map(|adapter| adapter.effective_ceiling())
        .collect();
    let ceiling = levels
        .iter()
        .copied()
        .fold(Classification::Public, Classification::high_water);

    if levels.len() > 1 && !entitlements.accepts_high_water {
        return Err(SelectionError::MixedLevels {
            levels: levels.into_iter().collect(),
            ceiling,
        });
    }

    Ok(Selection {
        adapters: eligible
            .iter()
            .map(|adapter| adapter.file.clone())
            .collect(),
        ceiling,
    })
}

fn is_entitled(adapter: &ComponentRecord, entitlements: &Entitlements) -> bool {
    if adapter.effective_ceiling() > entitlements.clearance {
        return false;
    }
    match adapter.role.compartment() {
        Some(compartment) => entitlements.compartments.contains(compartment),
        // An adapter with no compartment serves anyone cleared for its level.
        None => true,
    }
}
