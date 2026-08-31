//! Taint-aware egress gate (SEQ-003, SEQ-014).
//!
//! The gate answers one question: may this payload, carrying this taint, leave
//! for this destination, given what the requester holds. It composes the
//! destination checks that already exist rather than replacing them — an SSRF
//! block and a taint block are different failures and both have to stand.
//!
//! Two properties are load-bearing.
//!
//! The gate never authorizes by itself in the sense of deciding what an
//! attribute means: it reads requirements out of the taxonomy map and compares
//! them to what the requester presents. Attribute semantics stay with the
//! policy decision point.
//!
//! What a refused caller is told is deliberately uniform. A denial that says
//! *why* turns the gate into a classifier oracle: an agent can binary-search a
//! payload against the reason string until it learns exactly what the detector
//! sees. The reason goes to the audit record; the caller gets one sentence.

use std::collections::BTreeSet;

use governor::{Quota, RateLimiter, clock::DefaultClock, state::InMemoryState, state::NotKeyed};
use std::num::NonZeroU32;

use crate::data_classification::{DataCategory, DlpAction, SensitivityLevel};
use crate::egress_destination::{Destination, DestinationPolicy};
use crate::taint::{AGGREGATE_SOURCE_ID, TaintSet};
use crate::taxonomy::{AttributeRequirement, TaxonomyMap};

/// What a refused caller is told, whatever the reason.
pub const GENERIC_DENIAL: &str = "egress refused by data policy";
/// What a held caller is told.
pub const GENERIC_HOLD: &str = "egress held pending policy resolution";

/// What the requesting subject presents.
///
/// Bare by default: a requester that asserts nothing holds nothing, so the
/// conservative path is the one taken when a caller forgets to populate this.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequesterEntitlements {
    attributes: BTreeSet<AttributeRequirement>,
    did: Option<String>,
}

impl RequesterEntitlements {
    pub fn none() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_attribute(mut self, fqn: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes
            .insert(AttributeRequirement::new(fqn, value));
        self
    }

    #[must_use]
    pub fn with_did(mut self, did: impl Into<String>) -> Self {
        self.did = Some(did.into());
        self
    }

    pub fn did(&self) -> Option<&str> {
        self.did.as_deref()
    }

    /// Requirements the subject does not satisfy, in a stable order.
    ///
    /// Exact match on value. `clearance` is hierarchical in the OpenTDF
    /// definition, and expanding a hierarchy here would put a second, divergent
    /// copy of that rule in the gate; the requester presents the entitlements
    /// the decision point already resolved for them.
    pub fn missing(&self, required: &BTreeSet<AttributeRequirement>) -> Vec<AttributeRequirement> {
        required.difference(&self.attributes).cloned().collect()
    }
}

/// Why a payload was refused. Audit-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenialReason {
    /// A category the taxonomy marks as never releasable is present.
    NeverRelease { categories: Vec<DataCategory> },
    /// The requester does not hold what the taxonomy requires.
    NotEntitled { missing: Vec<AttributeRequirement> },
    /// The destination cannot consume a TDF, and the alternative to wrapping is
    /// shipping plaintext, which is not an alternative.
    DestinationCannotWrap { destination: String },
    /// The gate allowed delivery under a wrap this caller cannot perform. The
    /// destination is not at fault — nothing on this path can produce a TDF, so
    /// the only way to proceed would be to send the plaintext the wrap exists
    /// to prevent.
    NoWrapPath {
        attributes: Vec<AttributeRequirement>,
    },
    /// The destination checks that predate taint refused it.
    Destination { detail: String },
    /// Provenance for this payload is incomplete, so the gate cannot show the
    /// payload is safe to release (SEQ-014 edge case: tracking gap).
    ProvenanceIncomplete { detail: String },
}

impl DenialReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            DenialReason::NeverRelease { .. } => "never_release",
            DenialReason::NotEntitled { .. } => "not_entitled",
            DenialReason::DestinationCannotWrap { .. } => "destination_cannot_wrap",
            DenialReason::NoWrapPath { .. } => "no_wrap_path",
            DenialReason::Destination { .. } => "destination_blocked",
            DenialReason::ProvenanceIncomplete { .. } => "provenance_incomplete",
        }
    }

    /// Full detail, for the audit record only.
    pub fn audit_detail(&self) -> String {
        match self {
            DenialReason::NeverRelease { categories } => format!(
                "categories marked never-release: {}",
                categories
                    .iter()
                    .map(|c| format!("{c:?}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            DenialReason::NotEntitled { missing } => format!(
                "requester lacks: {}",
                missing
                    .iter()
                    .map(|a| format!("{}={}", a.fqn, a.value))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            DenialReason::DestinationCannotWrap { destination } => {
                format!("{destination} cannot consume a wrapped payload")
            }
            DenialReason::NoWrapPath { attributes } => format!(
                "delivery required a wrap under {} and this path cannot produce one",
                attributes
                    .iter()
                    .map(|a| format!("{}={}", a.fqn, a.value))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            DenialReason::Destination { detail } => detail.clone(),
            DenialReason::ProvenanceIncomplete { detail } => detail.clone(),
        }
    }
}

/// Why a payload was held rather than refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldReason {
    /// The gate could not resolve where this was going.
    DestinationUnresolved { hint: String },
    /// Evaluation budget is exhausted. Held rather than allowed: a flood must
    /// not be a way to get past the gate.
    RateLimited,
}

impl HoldReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            HoldReason::DestinationUnresolved { .. } => "destination_unresolved",
            HoldReason::RateLimited => "rate_limited",
        }
    }
}

/// What the gate decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EgressDisposition {
    Allow,
    /// Deliver, but only wrapped under these attributes.
    Wrap {
        attributes: Vec<AttributeRequirement>,
        dissemination: Vec<String>,
    },
    Hold(HoldReason),
    Block(DenialReason),
}

impl EgressDisposition {
    pub fn as_str(&self) -> &'static str {
        match self {
            EgressDisposition::Allow => "allow",
            EgressDisposition::Wrap { .. } => "wrap",
            EgressDisposition::Hold(_) => "hold",
            EgressDisposition::Block(_) => "block",
        }
    }

    /// Whether the payload may go out as it stands.
    ///
    /// `Wrap` is deliberately not included. It says the payload may travel
    /// *wrapped*; a caller that cannot wrap and reads this as permission sends
    /// the plaintext the wrap existed to prevent. Naming the question after the
    /// plaintext makes that misreading hard to write.
    pub fn may_send_plaintext(&self) -> bool {
        matches!(self, EgressDisposition::Allow)
    }

    /// Whether the payload may travel at all, in some form.
    pub fn permits_delivery(&self) -> bool {
        matches!(
            self,
            EgressDisposition::Allow | EgressDisposition::Wrap { .. }
        )
    }

    /// The same decision in the DLP vocabulary, for callers that speak it.
    pub fn as_dlp_action(&self) -> DlpAction {
        match self {
            EgressDisposition::Allow => DlpAction::Allow,
            EgressDisposition::Wrap { attributes, .. } => DlpAction::Wrap {
                attributes: attributes
                    .iter()
                    .map(|a| (a.fqn.clone(), a.value.clone()))
                    .collect(),
            },
            EgressDisposition::Hold(_) => DlpAction::Hold,
            EgressDisposition::Block(_) => DlpAction::Block,
        }
    }

    /// What the caller is allowed to learn. Uniform across reasons.
    pub fn public_message(&self) -> Option<&'static str> {
        match self {
            EgressDisposition::Allow | EgressDisposition::Wrap { .. } => None,
            EgressDisposition::Hold(_) => Some(GENERIC_HOLD),
            EgressDisposition::Block(_) => Some(GENERIC_DENIAL),
        }
    }
}

/// Everything an auditor needs to reconstruct the decision (SEQ-014, SEQ-015).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressEvidence {
    pub sensitivity: SensitivityLevel,
    pub categories: Vec<DataCategory>,
    pub sources: Vec<String>,
    pub provenance: Vec<String>,
    pub truncated_hops: u32,
    pub destination: String,
    pub taxonomy_version: String,
}

impl EgressEvidence {
    fn from_taint(taint: &TaintSet, destination: &Destination, taxonomy: &TaxonomyMap) -> Self {
        let mut provenance = Vec::new();
        let mut truncated_hops = 0u32;
        for label in taint.labels() {
            truncated_hops = truncated_hops.saturating_add(label.truncated_hops);
            for hop in &label.hops {
                provenance.push(format!(
                    "{}|{}|{}",
                    label.source_id,
                    hop.transformation.as_str(),
                    hop.detail
                ));
            }
        }
        Self {
            sensitivity: taint.sensitivity(),
            categories: taint.categories().into_iter().collect(),
            sources: taint.source_ids().map(str::to_string).collect(),
            provenance,
            truncated_hops,
            destination: destination.audit_detail(),
            taxonomy_version: taxonomy.version().to_string(),
        }
    }

    /// SEQ-014: the provenance chain, rendered for a denial audit event.
    pub fn provenance_chain(&self) -> String {
        if self.provenance.is_empty() {
            return format!(
                "provenance: {} (no transformations)",
                self.sources.join(",")
            );
        }
        format!("provenance: {}", self.provenance.join(" -> "))
    }
}

/// The gate's answer plus the evidence behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressDecision {
    pub disposition: EgressDisposition,
    pub evidence: EgressEvidence,
}

impl EgressDecision {
    /// Whether this payload may go out as it stands. See
    /// [`EgressDisposition::may_send_plaintext`].
    pub fn may_send_plaintext(&self) -> bool {
        self.disposition.may_send_plaintext()
    }

    pub fn permits_delivery(&self) -> bool {
        self.disposition.permits_delivery()
    }

    /// What the caller is told. Never names a category, a source, or a reason.
    pub fn public_message(&self) -> Option<&'static str> {
        self.disposition.public_message()
    }

    /// SEQ-014: what the audit sink is told, which is everything.
    pub fn audit_detail(&self) -> String {
        let reason = match &self.disposition {
            EgressDisposition::Block(reason) => reason.audit_detail(),
            EgressDisposition::Hold(reason) => reason.as_str().to_string(),
            EgressDisposition::Allow => "released".to_string(),
            EgressDisposition::Wrap { attributes, .. } => format!(
                "wrapped under {}",
                attributes
                    .iter()
                    .map(|a| format!("{}={}", a.fqn, a.value))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        };
        format!(
            "{} to {}: {} [{}]",
            self.disposition.as_str(),
            self.evidence.destination,
            reason,
            self.evidence.provenance_chain()
        )
    }
}

type DirectLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// Evaluates payload taint against a destination.
pub struct EgressTaintGate {
    filter: arkavo_validation::EgressFilter,
    taxonomy: &'static TaxonomyMap,
    destinations: DestinationPolicy,
    limiter: Option<DirectLimiter>,
}

impl Default for EgressTaintGate {
    fn default() -> Self {
        Self::new()
    }
}

impl EgressTaintGate {
    pub fn new() -> Self {
        Self {
            filter: arkavo_validation::EgressFilter::new(),
            taxonomy: TaxonomyMap::v1(),
            destinations: DestinationPolicy::new(),
            limiter: None,
        }
    }

    #[must_use]
    pub fn with_destination_policy(mut self, destinations: DestinationPolicy) -> Self {
        self.destinations = destinations;
        self
    }

    #[must_use]
    pub fn with_egress_filter(mut self, filter: arkavo_validation::EgressFilter) -> Self {
        self.filter = filter;
        self
    }

    /// Bound how often full evaluation runs.
    ///
    /// # Panics
    ///
    /// Panics if `per_second` or `burst` is zero; a limiter that permits nothing
    /// would hold every call, which is a misconfiguration rather than a policy.
    #[must_use]
    pub fn with_rate_limit(mut self, per_second: u32, burst: u32) -> Self {
        let quota = Quota::per_second(NonZeroU32::new(per_second).expect("per_second must be > 0"))
            .allow_burst(NonZeroU32::new(burst).expect("burst must be > 0"));
        self.limiter = Some(RateLimiter::direct(quota));
        self
    }

    pub fn destinations(&self) -> &DestinationPolicy {
        &self.destinations
    }

    pub fn taxonomy(&self) -> &TaxonomyMap {
        self.taxonomy
    }

    /// SEQ-003: decide whether this payload may go to this destination.
    pub fn evaluate(
        &self,
        taint: &TaintSet,
        destination: &Destination,
        requester: &RequesterEntitlements,
    ) -> EgressDecision {
        let evidence = EgressEvidence::from_taint(taint, destination, self.taxonomy);
        let disposition = self.decide(taint, destination, requester, &evidence);
        EgressDecision {
            disposition,
            evidence,
        }
    }

    fn decide(
        &self,
        taint: &TaintSet,
        destination: &Destination,
        requester: &RequesterEntitlements,
        evidence: &EgressEvidence,
    ) -> EgressDisposition {
        if let Some(limiter) = &self.limiter
            && limiter.check().is_err()
        {
            return EgressDisposition::Hold(HoldReason::RateLimited);
        }

        let categories = taint.categories();

        // The checks that predate taint still stand on their own.
        if let Destination::Internal { url } | Destination::External { url } = destination
            && let Err(e) = self.filter.is_allowed(url)
        {
            return EgressDisposition::Block(DenialReason::Destination {
                detail: e.to_string(),
            });
        }

        if let Destination::Unresolved { hint } = destination {
            return EgressDisposition::Hold(HoldReason::DestinationUnresolved {
                hint: hint.clone(),
            });
        }

        // Inside the boundary the taint travels with the data and the same
        // policy governs the next hop, so no release decision is due here.
        //
        // This precedes the never-release check on purpose. An agent that
        // legitimately read a credential already holds it; refusing to let it
        // write inside its own workspace, or reach the sanctioned endpoint it
        // read from, would stop ordinary work while denying an attacker
        // nothing. "Unconditional" in SEQ-003 governs release, and a write that
        // does not cross the boundary is not one.
        if !destination.is_external() {
            return EgressDisposition::Allow;
        }

        // From here the data is leaving. A credential on that path is already
        // compromised, so neither entitlement nor wrapping is an answer.
        let never = self
            .taxonomy
            .never_release_categories(categories.iter().copied());
        if !never.is_empty() {
            return EgressDisposition::Block(DenialReason::NeverRelease { categories: never });
        }

        if let Some(detail) = provenance_gap(taint, evidence) {
            return EgressDisposition::Block(DenialReason::ProvenanceIncomplete { detail });
        }

        // Nothing sensitive is leaving.
        if taint.sensitivity() <= SensitivityLevel::Public
            && categories.iter().all(|c| *c == DataCategory::Public)
        {
            return EgressDisposition::Allow;
        }

        let mut required = self.taxonomy.requirements_for(categories.iter().copied());
        // Sensitivity alone carries a requirement. Without this a payload the
        // detector found no category in — the common case, since ingestion
        // applies a floor whether or not anything matched — would require
        // nothing, satisfy every subject vacuously, and be wrapped under an
        // empty attribute set.
        if let Some(clearance) = self.taxonomy.clearance_requirement(taint.sensitivity()) {
            required.insert(clearance);
        }
        let missing = requester.missing(&required);
        if !missing.is_empty() {
            // Wrapping does not rescue an unauthorized disclosure: the wrapped
            // payload plus a key request is still the payload.
            return EgressDisposition::Block(DenialReason::NotEntitled { missing });
        }

        if !self.destinations.can_consume_tdf(destination) {
            return EgressDisposition::Block(DenialReason::DestinationCannotWrap {
                destination: destination.audit_detail(),
            });
        }

        EgressDisposition::Wrap {
            attributes: self
                .taxonomy
                .wrap_attributes_for(categories.iter().copied())
                .into_iter()
                .collect(),
            dissemination: requester.did().map(str::to_string).into_iter().collect(),
        }
    }
}

/// Whether provenance is too incomplete to justify a release.
///
/// Both conditions mean the same thing operationally: the chain the gate would
/// have to defend in an audit is not the chain that happened.
fn provenance_gap(taint: &TaintSet, evidence: &EgressEvidence) -> Option<String> {
    if evidence.truncated_hops > 0 {
        return Some(format!(
            "{} provenance hops were dropped before this decision",
            evidence.truncated_hops
        ));
    }
    if taint.label_for(AGGREGATE_SOURCE_ID).is_some() {
        return Some("source labels were folded into an aggregate".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taint::{ProvenanceHop, TaintLabel, Transformation};

    const CLEARANCE: &str = "https://attr.arkavo.com/clearance";
    const DEPARTMENT: &str = "https://attr.arkavo.com/department";

    fn tainted(category: DataCategory, level: SensitivityLevel) -> TaintSet {
        TaintSet::from_label(TaintLabel::new("tool:read", [category], level))
    }

    fn external() -> Destination {
        Destination::External {
            url: "https://api.example.com/collect".into(),
        }
    }

    fn gate() -> EgressTaintGate {
        EgressTaintGate::new().with_destination_policy(
            DestinationPolicy::new()
                .sanction_host("vault.internal")
                .tdf_capable_host("api.example.com")
                .workspace_root("/work/agent"),
        )
    }

    #[test]
    fn credentials_are_blocked_however_entitled_the_requester() {
        let entitled = RequesterEntitlements::none()
            .with_attribute(CLEARANCE, "restricted")
            .with_did("did:web:example.com:agent:a");

        let decision = gate().evaluate(
            &tainted(DataCategory::Credentials, SensitivityLevel::Restricted),
            &external(),
            &entitled,
        );

        assert!(matches!(
            decision.disposition,
            EgressDisposition::Block(DenialReason::NeverRelease { .. })
        ));
    }

    #[test]
    fn internal_data_does_not_reach_an_external_endpoint_unentitled() {
        let decision = gate().evaluate(
            &tainted(DataCategory::Internal, SensitivityLevel::Internal),
            &external(),
            &RequesterEntitlements::none(),
        );

        assert!(matches!(
            decision.disposition,
            EgressDisposition::Block(DenialReason::NotEntitled { .. })
        ));
    }

    #[test]
    fn pii_needs_explicit_authorization_and_then_travels_wrapped() {
        let entitled = RequesterEntitlements::none()
            .with_attribute(CLEARANCE, "internal")
            .with_did("did:web:example.com:agent:a");

        let decision = gate().evaluate(
            &tainted(DataCategory::Pii, SensitivityLevel::Internal),
            &external(),
            &entitled,
        );

        match decision.disposition {
            EgressDisposition::Wrap {
                attributes,
                dissemination,
            } => {
                assert!(attributes.contains(&AttributeRequirement::new(CLEARANCE, "internal")));
                assert_eq!(dissemination, vec!["did:web:example.com:agent:a"]);
            }
            other => panic!("expected wrap, got {other:?}"),
        }
    }

    #[test]
    fn partial_entitlement_does_not_satisfy_a_conjunctive_requirement() {
        // Financial needs clearance AND department; clearance alone must not do.
        let half = RequesterEntitlements::none().with_attribute(CLEARANCE, "confidential");

        let decision = gate().evaluate(
            &tainted(DataCategory::Financial, SensitivityLevel::Confidential),
            &external(),
            &half,
        );

        match decision.disposition {
            EgressDisposition::Block(DenialReason::NotEntitled { missing }) => {
                assert_eq!(
                    missing,
                    vec![AttributeRequirement::new(DEPARTMENT, "finance")]
                );
            }
            other => panic!("expected a conjunctive denial, got {other:?}"),
        }
    }

    #[test]
    fn an_entitled_requester_cannot_downgrade_to_plaintext() {
        let entitled = RequesterEntitlements::none().with_attribute(CLEARANCE, "internal");
        let no_tdf = EgressTaintGate::new().with_destination_policy(DestinationPolicy::new());

        let decision = no_tdf.evaluate(
            &tainted(DataCategory::Pii, SensitivityLevel::Internal),
            &external(),
            &entitled,
        );

        assert!(matches!(
            decision.disposition,
            EgressDisposition::Block(DenialReason::DestinationCannotWrap { .. })
        ));
    }

    #[test]
    fn a_credential_may_be_written_inside_the_workspace() {
        // Regression: the never-release check ran before the boundary test, so
        // an agent that had read any secret lost permission to write inside its
        // own workspace.
        let decision = gate().evaluate(
            &tainted(DataCategory::Credentials, SensitivityLevel::Restricted),
            &Destination::Workspace {
                path: "/work/agent/notes.md".into(),
            },
            &RequesterEntitlements::none(),
        );

        assert_eq!(decision.disposition, EgressDisposition::Allow);
    }

    #[test]
    fn a_credential_may_reach_the_sanctioned_endpoint_it_came_from() {
        let decision = gate().evaluate(
            &tainted(DataCategory::Credentials, SensitivityLevel::Restricted),
            &Destination::Internal {
                url: "https://vault.internal/rotate".into(),
            },
            &RequesterEntitlements::none(),
        );

        assert_eq!(decision.disposition, EgressDisposition::Allow);
    }

    #[test]
    fn a_credential_still_cannot_cross_the_boundary_by_file() {
        let decision = gate().evaluate(
            &tainted(DataCategory::Credentials, SensitivityLevel::Restricted),
            &Destination::ExternalPath {
                path: "/etc/cron.d/exfil".into(),
            },
            &RequesterEntitlements::none(),
        );

        assert!(matches!(
            decision.disposition,
            EgressDisposition::Block(DenialReason::NeverRelease { .. })
        ));
    }

    #[test]
    fn an_unresolved_destination_is_held_not_allowed() {
        let decision = gate().evaluate(
            &tainted(DataCategory::Internal, SensitivityLevel::Internal),
            &Destination::Unresolved { hint: "s3".into() },
            &RequesterEntitlements::none(),
        );

        assert!(matches!(
            decision.disposition,
            EgressDisposition::Hold(HoldReason::DestinationUnresolved { .. })
        ));
    }

    #[test]
    fn a_sanctioned_internal_endpoint_proceeds() {
        let decision = gate().evaluate(
            &tainted(DataCategory::Internal, SensitivityLevel::Internal),
            &Destination::Internal {
                url: "https://vault.internal/store".into(),
            },
            &RequesterEntitlements::none(),
        );

        assert_eq!(decision.disposition, EgressDisposition::Allow);
    }

    #[test]
    fn public_data_reaches_a_public_api() {
        let decision = gate().evaluate(
            &TaintSet::from_label(TaintLabel::new(
                "tool:docs",
                [DataCategory::Public],
                SensitivityLevel::Public,
            )),
            &external(),
            &RequesterEntitlements::none(),
        );

        assert_eq!(decision.disposition, EgressDisposition::Allow);
    }

    #[test]
    fn an_allowlisted_destination_does_not_override_taint() {
        // SEQ-014 edge case: the destination allowlist answers a different
        // question than the payload's classification does.
        let mut filter = arkavo_validation::EgressFilter::new();
        filter.allow("https://api.example.com/collect");
        let gate = gate().with_egress_filter(filter);

        let decision = gate.evaluate(
            &tainted(DataCategory::Internal, SensitivityLevel::Internal),
            &external(),
            &RequesterEntitlements::none(),
        );

        assert!(matches!(
            decision.disposition,
            EgressDisposition::Block(DenialReason::NotEntitled { .. })
        ));
    }

    #[test]
    fn ssrf_targets_still_fail_the_destination_check() {
        let decision = gate().evaluate(
            &TaintSet::new(),
            &Destination::External {
                url: "http://169.254.169.254/latest/meta-data".into(),
            },
            &RequesterEntitlements::none(),
        );

        assert!(matches!(
            decision.disposition,
            EgressDisposition::Block(DenialReason::Destination { .. })
        ));
    }

    #[test]
    fn a_truncated_provenance_chain_blocks_conservatively() {
        let mut label = TaintLabel::new(
            "tool:read",
            [DataCategory::Internal],
            SensitivityLevel::Internal,
        );
        for i in 0..crate::taint::MAX_PROVENANCE_HOPS + 1 {
            label.push_hop(ProvenanceHop::new(Transformation::Encode, format!("h{i}")));
        }
        let entitled = RequesterEntitlements::none().with_attribute(CLEARANCE, "internal");

        let decision = gate().evaluate(&TaintSet::from_label(label), &external(), &entitled);

        assert!(matches!(
            decision.disposition,
            EgressDisposition::Block(DenialReason::ProvenanceIncomplete { .. })
        ));
    }

    #[test]
    fn every_denial_reads_the_same_to_the_caller() {
        let gate = gate();
        let never = gate.evaluate(
            &tainted(DataCategory::Credentials, SensitivityLevel::Restricted),
            &external(),
            &RequesterEntitlements::none(),
        );
        let unentitled = gate.evaluate(
            &tainted(DataCategory::Financial, SensitivityLevel::Confidential),
            &external(),
            &RequesterEntitlements::none(),
        );

        assert_eq!(never.public_message(), Some(GENERIC_DENIAL));
        assert_eq!(unentitled.public_message(), Some(GENERIC_DENIAL));
        // ...while the audit records stay distinguishable.
        assert_ne!(never.audit_detail(), unentitled.audit_detail());
    }

    #[test]
    fn the_audit_record_carries_the_provenance_chain() {
        let taint = tainted(DataCategory::Financial, SensitivityLevel::Confidential)
            .transformed(Transformation::Encode, "base64");

        let decision = gate().evaluate(&taint, &external(), &RequesterEntitlements::none());

        assert!(
            decision.audit_detail().contains("encode|base64"),
            "provenance missing from audit record: {}",
            decision.audit_detail()
        );
    }

    #[test]
    fn an_exhausted_evaluation_budget_holds_rather_than_allows() {
        let gate = gate().with_rate_limit(1, 1);
        let taint = TaintSet::from_label(TaintLabel::new(
            "tool:docs",
            [DataCategory::Public],
            SensitivityLevel::Public,
        ));

        let mut held = false;
        for _ in 0..8 {
            if matches!(
                gate.evaluate(&taint, &external(), &RequesterEntitlements::none())
                    .disposition,
                EgressDisposition::Hold(HoldReason::RateLimited)
            ) {
                held = true;
                break;
            }
        }

        assert!(held, "a flood of calls was never throttled");
    }

    #[test]
    fn dispositions_map_onto_the_dlp_vocabulary() {
        let entitled = RequesterEntitlements::none().with_attribute(CLEARANCE, "internal");
        let wrap = gate()
            .evaluate(
                &tainted(DataCategory::Pii, SensitivityLevel::Internal),
                &external(),
                &entitled,
            )
            .disposition;

        assert!(matches!(wrap.as_dlp_action(), DlpAction::Wrap { .. }));
        assert_eq!(
            EgressDisposition::Hold(HoldReason::RateLimited).as_dlp_action(),
            DlpAction::Hold
        );
    }
}
