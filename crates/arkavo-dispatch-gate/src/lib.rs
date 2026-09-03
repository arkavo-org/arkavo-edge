//! The dispatch gate: authn (permit signature, window, proof-of-possession),
//! policy (bundle hash, tool and argument binding), budget (invocations per
//! permit). Local crypto only, no I/O, so it fits inside the 25ms budget
//! documented in `docs/gate-latency-baseline.md`. Sequence integrity and
//! step-up are later stages and plug in before `Allow` is returned.
//!
//! The budget counter is keyed on `Permit::id`, the digest of the permit's
//! signed content, never on the token bytes: one issuance has many valid
//! encodings, so a byte-keyed counter would hand the holder a fresh budget
//! for every re-encoding.
//!
//! Budget is spent when a call is admitted, which is before the call runs.
//! A dispatcher that can prove its upstream never received the call returns
//! the invocation with [`DispatchGate::refund`], so a failure the holder
//! never benefited from does not exhaust a permit; a call the upstream ran —
//! or may have run, which includes one that timed out — keeps its
//! invocation, whatever it answered.
//!
//! `GateConfig::trusted_issuers` forms one trust domain: authn passes for a
//! permit signed by any listed issuer, with no per-issuer policy and no
//! binding to the permit's `iss` claim yet. `arkavo_permit::decode` must
//! never be used for authn — it checks neither the issuer nor the
//! signature, only claim structure.

use arkavo_permit::{HashAlgorithm, PermitVerifier, verify, verify_invocation_proof};
use serde_json::Value;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt;
use std::sync::Mutex;

// The proxy in front of this gate bounds each credential it decodes by what
// that credential can be: a permit by the cap the permit parser enforces, a
// proof by the length of the one signature it holds. Both facts belong to
// `arkavo-permit`, so both are re-exported here rather than restated in a
// proxy that would otherwise need its own dependency on that crate to say
// something it does not own.
pub use arkavo_permit::{MAX_PERMIT_BYTES, SIGNATURE_BYTES};

/// The largest serialized `arguments` object the gate will hash.
///
/// Arguments are canonicalized and hashed twice per call — once for the
/// proof-of-possession digest, once for the permit's binding — so their size
/// is work the call asks for. The check sits after the permit's signature has
/// verified, so that work is only ever reachable by a caller holding a permit
/// from a trusted issuer; the bound is what keeps even that caller from
/// turning one admitted call into an unbounded hash. MCP tool arguments are
/// small; a quarter of a megabyte is far above any real call.
pub const MAX_ARGUMENTS_BYTES: usize = 256 * 1024;

pub struct GateConfig {
    pub policy_bundle_hash: Vec<u8>,
    pub hash: HashAlgorithm,
    pub clock: fn() -> i64,
    pub trusted_issuers: Vec<PermitVerifier>,
}

pub struct GateRequest<'a> {
    pub tool_name: &'a str,
    pub arguments: &'a Value,
    pub permit: &'a [u8],
    pub proof: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Authn,
    Policy,
    Budget,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Authn => "authn",
            Self::Policy => "policy",
            Self::Budget => "budget",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateDecision {
    Allow {
        permit_id: [u8; 32],
        subject: String,
    },
    Deny {
        stage: Stage,
        reason: String,
    },
}

struct Usage {
    invocations: u64,
    expires_at: i64,
}

/// The usage table and the bookkeeping that decides when it is worth
/// pruning, all behind one lock so the two never drift apart.
struct UsageTable {
    entries: HashMap<[u8; 32], Usage>,
    /// The clock second of the table's most recent expiry scan.
    /// `i64::MIN` before the first one, so a table that has never pruned
    /// always prunes the first time it is found full.
    last_pruned_at: i64,
    /// Incremented once per expiry scan that actually runs. Nothing outside
    /// tests reads it; it exists so a test can observe that a scan was
    /// skipped rather than inferring it from timing.
    #[cfg(test)]
    prune_count: u64,
}

impl UsageTable {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            last_pruned_at: i64::MIN,
            #[cfg(test)]
            prune_count: 0,
        }
    }
}

/// How many permits the usage table counts at once.
///
/// Counters are keyed by `Permit::id`, and a caller can mint arbitrarily many
/// permits, so the table has to be bounded. Only *expired* counters are ever
/// dropped: evicting a live one would restart a still-valid permit's
/// invocation count at zero, which is exactly the budget a flood of fresh
/// permits would be trying to buy. When pruning frees nothing, the gate fails
/// closed instead — a permit the table is not already counting is denied at
/// [`Stage::Budget`] until entries expire, while every permit already counted
/// keeps being counted normally.
const MAX_TRACKED_PERMITS: usize = 65_536;

pub struct DispatchGate {
    config: GateConfig,
    /// The most counters [`Self::usage`] holds; `MAX_TRACKED_PERMITS` outside
    /// tests, which use a small table to reach the limit cheaply.
    capacity: usize,
    usage: Mutex<UsageTable>,
}

impl DispatchGate {
    pub fn new(config: GateConfig) -> Self {
        Self::with_capacity(config, MAX_TRACKED_PERMITS)
    }

    /// A gate whose usage table counts at most `capacity` permits at once.
    fn with_capacity(config: GateConfig, capacity: usize) -> Self {
        Self {
            config,
            capacity,
            usage: Mutex::new(UsageTable::new()),
        }
    }

    pub fn evaluate(&self, request: &GateRequest<'_>) -> GateDecision {
        let now = (self.config.clock)();

        let permit = match verify(request.permit, now, &self.config.trusted_issuers) {
            Ok(permit) => permit,
            Err(error) => return deny(Stage::Authn, error.to_string()),
        };
        // Before the arguments are canonicalized and hashed — twice, below —
        // bound the work they can ask for. This sits after the permit's
        // signature so only a caller holding a trusted permit can reach it,
        // and before the proof, which is the first thing to hash them.
        if let Some(size) = oversized_arguments(request.arguments) {
            return deny(
                Stage::Policy,
                format!("arguments of {size} bytes exceed the {MAX_ARGUMENTS_BYTES} byte limit"),
            );
        }
        if let Err(error) = verify_invocation_proof(
            &permit,
            request.tool_name,
            request.arguments,
            request.proof,
            self.config.hash,
        ) {
            return deny(Stage::Authn, error.to_string());
        }

        if permit.claims.policy_bundle_hash != self.config.policy_bundle_hash {
            return deny(
                Stage::Policy,
                "permit was issued under a different policy bundle".into(),
            );
        }
        if let Err(error) =
            permit
                .claims
                .verify_invocation(request.tool_name, request.arguments, self.config.hash)
        {
            return deny(Stage::Policy, error.to_string());
        }

        let permit_id = permit.id;
        let mut usage = self
            .usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // A full table drops its expired counters, which cost nothing: the
        // permits they belong to are refused at authn from now on. The scan
        // is O(capacity), so once the table is full it runs at most once per
        // clock second rather than on every call — most calls that find the
        // table full find it still full a moment later, and re-scanning for
        // them buys nothing. Skipping the scan still falls through to the
        // same capacity decision below, just without paying for a retain
        // that would have freed no room.
        if usage.entries.len() >= self.capacity && now > usage.last_pruned_at {
            usage.entries.retain(|_, entry| entry.expires_at > now);
            usage.last_pruned_at = now;
            #[cfg(test)]
            {
                usage.prune_count += 1;
            }
        }
        let full = usage.entries.len() >= self.capacity;
        let entry = match usage.entries.entry(permit_id) {
            Entry::Occupied(counted) => counted.into_mut(),
            // Nothing is evicted to make room. A live counter reset to zero
            // would hand its permit a second budget, so a permit this gate is
            // not already counting waits instead.
            Entry::Vacant(_) if full => {
                return deny(
                    Stage::Budget,
                    "gate capacity exhausted; retry after permits expire".into(),
                );
            }
            Entry::Vacant(slot) => slot.insert(Usage {
                invocations: 0,
                expires_at: permit.claims.expires_at,
            }),
        };
        if entry.invocations >= permit.claims.budget.max_invocations {
            return deny(
                Stage::Budget,
                format!(
                    "invocation budget of {} exhausted",
                    permit.claims.budget.max_invocations
                ),
            );
        }
        // Must stay the last step before `Allow`: no stage runs after this
        // one, so a later addition must not be inserted below it, or it
        // could consume budget on a request this function goes on to deny.
        entry.invocations += 1;
        drop(usage);

        GateDecision::Allow {
            permit_id,
            subject: permit.claims.subject,
        }
    }

    /// Return one invocation to a permit's budget.
    ///
    /// The counter is spent when the gate admits a call, which is before the
    /// call runs. A dispatcher whose upstream never received the call — the
    /// connection was already down, the request never made it onto the wire —
    /// hands the invocation back with this, so a permit with a small budget is
    /// not exhausted by failures its holder never benefited from.
    ///
    /// Everything else keeps its invocation, including the two cases that
    /// look like failures from the caller's side: a call the upstream ran and
    /// answered with an error is a completed call, and a call that timed out
    /// was dispatched and may still be running there. Refunding a timeout
    /// would let any tool slower than the dispatcher's timeout be invoked
    /// without ever depleting a budget.
    ///
    /// Returns whether a counter was found and decremented. Never goes below
    /// zero, and never creates a counter: refunding a permit that never spent
    /// anything does nothing.
    pub fn refund(&self, permit_id: [u8; 32]) -> bool {
        let mut usage = self
            .usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match usage.entries.get_mut(&permit_id) {
            Some(entry) if entry.invocations > 0 => {
                entry.invocations -= 1;
                true
            }
            _ => false,
        }
    }

    /// Refund one invocation of the permit `permit` encodes.
    ///
    /// The permit is verified again rather than decoded, so the identity a
    /// refund is aimed at can only ever be one the caller can actually
    /// present: `decode` would let anyone who has seen a permit's signed
    /// content name it. A permit that has since expired is not refunded,
    /// which costs nothing — its budget is unusable either way.
    pub fn refund_invocation(&self, permit: &[u8]) -> bool {
        match verify(permit, (self.config.clock)(), &self.config.trusted_issuers) {
            Ok(permit) => self.refund(permit.id),
            Err(_) => false,
        }
    }

    #[cfg(test)]
    fn usage_len(&self) -> usize {
        self.usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries
            .len()
    }

    /// How many times the usage table has actually run its expiry scan.
    #[cfg(test)]
    fn prune_count(&self) -> u64 {
        self.usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .prune_count
    }
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().cast_signed())
        .unwrap_or(0)
}

/// The serialized size of `arguments` when it is over the cap, or `None`.
fn oversized_arguments(arguments: &Value) -> Option<usize> {
    // `Value` cannot fail to serialize; treating a failure as oversized keeps
    // the check fail-closed regardless.
    let size = serde_json::to_vec(arguments).map_or(usize::MAX, |bytes| bytes.len());
    (size > MAX_ARGUMENTS_BYTES).then_some(size)
}

fn deny(stage: Stage, reason: String) -> GateDecision {
    GateDecision::Deny { stage, reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;
    use arkavo_permit::{
        Budget, PermitClaims, PermitSigner, argument_hash, mint, prove_invocation,
    };
    use arkavo_test_macros::spec;
    use coset::{CborSerializable, CoseSign1, TaggedCborSerializable};
    use serde_json::json;

    const NOW: i64 = 1_700_000_060;
    fn clock() -> i64 {
        NOW
    }

    fn gate() -> (DispatchGate, PermitSigner) {
        let issuer = PermitSigner::Ed25519(AgentKeypair::generate());
        let dispatch_gate = DispatchGate::new(GateConfig {
            policy_bundle_hash: vec![7; 32],
            hash: HashAlgorithm::Sha256,
            clock,
            trusted_issuers: vec![issuer.public_key()],
        });
        (dispatch_gate, issuer)
    }

    fn permit(
        issuer: &PermitSigner,
        holder: &PermitSigner,
        tool: &str,
        args: &serde_json::Value,
        max: u64,
        exp: i64,
        bundle: u8,
    ) -> Vec<u8> {
        let claims = PermitClaims {
            issuer: "edge".into(),
            subject: "agent-1".into(),
            expires_at: exp,
            not_before: NOW - 60,
            issued_at: NOW - 60,
            agent_workload_id: "wl-1".into(),
            policy_bundle_hash: vec![bundle; 32],
            tool_name: tool.into(),
            argument_hash: argument_hash(args, HashAlgorithm::Sha256),
            data_classifications: vec![],
            budget: Budget {
                max_invocations: max,
                token_ceiling: None,
                cost_micro_usd: None,
            },
            sequence_state_hash: vec![9; 32],
            parent_permit: None,
        };
        mint(&claims, issuer, &holder.public_key()).unwrap()
    }

    /// Split a minted permit into its tag-61 prefix and the COSE_Sign1 the
    /// permit crate wrapped in tag 18.
    fn parts(cwt: &[u8]) -> CoseSign1 {
        assert_eq!(&cwt[..4], &[0xd8, 0x3d, 0xd2, 0x84], "tag 61 then tag 18");
        CoseSign1::from_tagged_slice(&cwt[2..]).expect("minted permit parses")
    }

    fn wrap(sign1: Vec<u8>) -> Vec<u8> {
        let mut cwt = vec![0xd8, 0x3d];
        cwt.extend_from_slice(&sign1);
        cwt
    }

    /// The same signed COSE_Sign1, serialized without its tag 18. Different
    /// bytes, identical signed content — the shared parser accepts both.
    fn reencoded_bare(cwt: &[u8]) -> Vec<u8> {
        wrap(parts(cwt).to_vec().expect("re-encodes"))
    }

    /// The same signed COSE_Sign1 with a junk entry in the unprotected
    /// header, which is outside the signature and so still verifies.
    fn with_unprotected_entry(cwt: &[u8]) -> Vec<u8> {
        let mut sign1 = parts(cwt);
        sign1.unprotected.rest.push((
            coset::Label::Int(-1000),
            coset::cbor::value::Value::Text("padding".into()),
        ));
        wrap(sign1.to_tagged_vec().expect("re-encodes"))
    }

    /// The permit's identity, which is what a proof-of-possession names.
    /// A holder may take it from `decode`: naming their own permit is not an
    /// authorization decision, and the gate re-derives it from `verify`.
    fn permit_id(cwt: &[u8]) -> [u8; 32] {
        arkavo_permit::decode(cwt)
            .expect("a minted permit decodes")
            .id
    }

    fn call<'a>(
        tool: &'a str,
        args: &'a serde_json::Value,
        cwt: &'a [u8],
        proof: &'a [u8],
    ) -> GateRequest<'a> {
        GateRequest {
            tool_name: tool,
            arguments: args,
            permit: cwt,
            proof,
        }
    }

    #[test]
    fn valid_permit_and_proof_allow() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&issuer, &holder, "merge", &args, 2, NOW + 300, 7);
        let proof = prove_invocation(
            &holder,
            &permit_id(&cwt),
            "merge",
            &args,
            HashAlgorithm::Sha256,
        );
        match gate.evaluate(&call("merge", &args, &cwt, &proof)) {
            GateDecision::Allow { subject, .. } => assert_eq!(subject, "agent-1"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    #[spec("PDG-005")]
    fn expired_permit_is_denied_at_authn() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({});
        let cwt = permit(&issuer, &holder, "merge", &args, 2, NOW - 1, 7);
        let proof = prove_invocation(
            &holder,
            &permit_id(&cwt),
            "merge",
            &args,
            HashAlgorithm::Sha256,
        );
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Authn,
                ..
            }
        ));
    }

    #[test]
    #[spec("PDG-003")]
    fn replay_with_different_args_is_denied_at_authn() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&issuer, &holder, "merge", &args, 2, NOW + 300, 7);
        let proof = prove_invocation(
            &holder,
            &permit_id(&cwt),
            "merge",
            &args,
            HashAlgorithm::Sha256,
        );
        let other = json!({"pr": 2});
        assert!(matches!(
            gate.evaluate(&call("merge", &other, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Authn,
                ..
            }
        ));
    }

    #[test]
    #[spec("PDG-003")]
    fn cross_agent_reuse_is_denied_at_authn() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let intruder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&issuer, &holder, "merge", &args, 2, NOW + 300, 7);
        let proof = prove_invocation(
            &intruder,
            &permit_id(&cwt),
            "merge",
            &args,
            HashAlgorithm::Sha256,
        );
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Authn,
                ..
            }
        ));
    }

    #[test]
    fn foreign_policy_bundle_is_denied_at_policy() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({});
        let cwt = permit(&issuer, &holder, "merge", &args, 2, NOW + 300, 8);
        let proof = prove_invocation(
            &holder,
            &permit_id(&cwt),
            "merge",
            &args,
            HashAlgorithm::Sha256,
        );
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Policy,
                ..
            }
        ));
    }

    #[test]
    #[spec("PDG-006")]
    fn budget_exhaustion_is_denied_at_budget() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({});
        let cwt = permit(&issuer, &holder, "merge", &args, 1, NOW + 300, 7);
        let proof = prove_invocation(
            &holder,
            &permit_id(&cwt),
            "merge",
            &args,
            HashAlgorithm::Sha256,
        );
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Allow { .. }
        ));
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Budget,
                ..
            }
        ));
    }

    #[test]
    #[spec("PDG-004")]
    fn reencoded_permit_shares_its_budget() {
        // One issuance, two byte strings. Keying the counter on the permit's
        // signed identity is what stops the second from buying a second
        // invocation off a budget of one.
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&issuer, &holder, "merge", &args, 1, NOW + 300, 7);
        let proof = prove_invocation(
            &holder,
            &permit_id(&cwt),
            "merge",
            &args,
            HashAlgorithm::Sha256,
        );
        let allowed = gate.evaluate(&call("merge", &args, &cwt, &proof));
        assert!(matches!(allowed, GateDecision::Allow { .. }), "{allowed:?}");

        let bare = reencoded_bare(&cwt);
        assert_ne!(bare, cwt, "the re-encoding must differ in bytes");
        // The proof names the permit by its signed identity, so the very same
        // proof travels with the re-encoded token: one issuance, one identity,
        // one budget counter.
        assert_eq!(permit_id(&bare), permit_id(&cwt));
        let denied = gate.evaluate(&call("merge", &args, &bare, &proof));
        assert!(
            matches!(
                denied,
                GateDecision::Deny {
                    stage: Stage::Budget,
                    ..
                }
            ),
            "{denied:?}"
        );
        assert_eq!(gate.usage_len(), 1, "both encodings share one counter");
        let GateDecision::Allow { permit_id, .. } = allowed else {
            unreachable!()
        };
        assert_eq!(
            permit_id,
            arkavo_permit::verify(&bare, NOW, &[issuer.public_key()])
                .unwrap()
                .id,
            "the decision reports the permit's identity, not a hash of bytes"
        );
    }

    #[test]
    fn unprotected_header_permit_is_denied_at_authn() {
        // The unprotected header is unsigned, so a permit carrying one is
        // refused outright rather than admitted on its still-valid signature.
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&issuer, &holder, "merge", &args, 2, NOW + 300, 7);
        let padded = with_unprotected_entry(&cwt);
        assert_ne!(padded, cwt);
        let proof = prove_invocation(
            &holder,
            &permit_id(&padded),
            "merge",
            &args,
            HashAlgorithm::Sha256,
        );
        let decision = gate.evaluate(&call("merge", &args, &padded, &proof));
        assert!(
            matches!(
                decision,
                GateDecision::Deny {
                    stage: Stage::Authn,
                    ..
                }
            ),
            "{decision:?}"
        );
        assert_eq!(gate.usage_len(), 0, "a denied permit consumes no budget");
    }

    #[test]
    #[spec("PDG-002")]
    fn self_minted_permit_is_denied_at_authn() {
        let (gate, _issuer) = gate();
        let rogue = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&rogue, &rogue, "merge", &args, 2, NOW + 300, 7);
        let proof = prove_invocation(
            &rogue,
            &permit_id(&cwt),
            "merge",
            &args,
            HashAlgorithm::Sha256,
        );
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Authn,
                ..
            }
        ));
    }

    /// The usage table is bounded, and filling it must not buy anyone budget.
    /// A permit the table is not already counting is denied at the budget
    /// stage rather than evicting a live counter — a counter reset to zero is
    /// a second budget for a permit that has already spent its first — and
    /// the room comes back as entries expire.
    #[test]
    fn a_full_usage_table_denies_new_permits_rather_than_evicting_live_ones() {
        use std::sync::atomic::{AtomicI64, Ordering};

        // The gate's clock is a plain `fn`, so "later" is a value this test
        // moves rather than a captured variable.
        static CLOCK: AtomicI64 = AtomicI64::new(NOW);
        fn moving_clock() -> i64 {
            CLOCK.load(Ordering::SeqCst)
        }
        CLOCK.store(NOW, Ordering::SeqCst);

        const CAPACITY: usize = 8;
        let issuer = PermitSigner::Ed25519(AgentKeypair::generate());
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let gate = DispatchGate::with_capacity(
            GateConfig {
                policy_bundle_hash: vec![7; 32],
                hash: HashAlgorithm::Sha256,
                clock: moving_clock,
                trusted_issuers: vec![issuer.public_key()],
            },
            CAPACITY,
        );
        // Distinct arguments make distinct permits, so each takes its own
        // counter. Each carries a budget of two.
        let mint_one = |n: i64, expires_at: i64| {
            let args = json!({"n": n});
            let cwt = permit(&issuer, &holder, "merge", &args, 2, expires_at, 7);
            let proof = prove_invocation(
                &holder,
                &permit_id(&cwt),
                "merge",
                &args,
                HashAlgorithm::Sha256,
            );
            (args, cwt, proof)
        };

        // Fill the table exactly: half the permits expire while the gate is
        // still running, half outlive the whole test.
        let filling: Vec<_> = (0..4)
            .map(|n| mint_one(n, NOW + 30))
            .chain((4..8).map(|n| mint_one(n, NOW + 3600)))
            .collect();
        for (args, cwt, proof) in &filling {
            let decision = gate.evaluate(&call("merge", args, cwt, proof));
            assert!(
                matches!(decision, GateDecision::Allow { .. }),
                "{decision:?}"
            );
        }
        assert_eq!(gate.usage_len(), CAPACITY, "the table is full");

        // A permit with no counter yet is refused, and refusing it leaves
        // every live counter where it was.
        let (new_args, new_cwt, new_proof) = mint_one(99, NOW + 3600);
        let denied = gate.evaluate(&call("merge", &new_args, &new_cwt, &new_proof));
        assert!(
            matches!(
                &denied,
                GateDecision::Deny {
                    stage: Stage::Budget,
                    reason,
                } if reason.contains("capacity")
            ),
            "{denied:?}"
        );
        assert_eq!(gate.usage_len(), CAPACITY, "nothing was evicted");

        // A permit the table already counts keeps being counted normally: it
        // spends its second invocation and is then out of budget, which is
        // exactly what an eviction would have undone.
        let (args, cwt, proof) = &filling[0];
        let second = gate.evaluate(&call("merge", args, cwt, proof));
        assert!(matches!(second, GateDecision::Allow { .. }), "{second:?}");
        let third = gate.evaluate(&call("merge", args, cwt, proof));
        assert!(
            matches!(
                &third,
                GateDecision::Deny {
                    stage: Stage::Budget,
                    reason,
                } if reason.contains("budget of 2")
            ),
            "a full table must not reset a live counter: {third:?}"
        );

        // Past the first four permits' expiry, pruning frees their counters —
        // they are refused at authn from here on — and the new permit fits.
        CLOCK.store(NOW + 60, Ordering::SeqCst);
        let admitted = gate.evaluate(&call("merge", &new_args, &new_cwt, &new_proof));
        assert!(
            matches!(admitted, GateDecision::Allow { .. }),
            "expired counters must make room: {admitted:?}"
        );
        assert_eq!(gate.usage_len(), 5, "four expired counters, one new permit");
    }

    /// The expiry scan is O(capacity), so once the table is full it must
    /// not re-run on every call that finds no room: it runs at most once
    /// per clock second, and repeat calls within that second skip straight
    /// to the capacity decision.
    #[test]
    fn a_full_table_prunes_at_most_once_per_clock_second() {
        use std::sync::atomic::{AtomicI64, Ordering};

        static CLOCK: AtomicI64 = AtomicI64::new(NOW);
        fn moving_clock() -> i64 {
            CLOCK.load(Ordering::SeqCst)
        }
        CLOCK.store(NOW, Ordering::SeqCst);

        const CAPACITY: usize = 4;
        let issuer = PermitSigner::Ed25519(AgentKeypair::generate());
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let gate = DispatchGate::with_capacity(
            GateConfig {
                policy_bundle_hash: vec![7; 32],
                hash: HashAlgorithm::Sha256,
                clock: moving_clock,
                trusted_issuers: vec![issuer.public_key()],
            },
            CAPACITY,
        );
        let mint_one = |n: i64, expires_at: i64| {
            let args = json!({"n": n});
            let cwt = permit(&issuer, &holder, "merge", &args, 2, expires_at, 7);
            let proof = prove_invocation(
                &holder,
                &permit_id(&cwt),
                "merge",
                &args,
                HashAlgorithm::Sha256,
            );
            (args, cwt, proof)
        };

        // Every live counter expires at the same instant, so the whole
        // table becomes prunable in one step once the clock passes it.
        let capacity = i64::try_from(CAPACITY).expect("a small test capacity fits in an i64");
        let filling: Vec<_> = (0..capacity).map(|n| mint_one(n, NOW + 30)).collect();
        for (args, cwt, proof) in &filling {
            let decision = gate.evaluate(&call("merge", args, cwt, proof));
            assert!(
                matches!(decision, GateDecision::Allow { .. }),
                "{decision:?}"
            );
        }
        assert_eq!(gate.usage_len(), CAPACITY, "the table is full");
        assert_eq!(gate.prune_count(), 0, "filling the table never scans it");

        let (new_args, new_cwt, new_proof) = mint_one(99, NOW + 3600);

        // At capacity, the first call this second scans for room and finds
        // none.
        let first = gate.evaluate(&call("merge", &new_args, &new_cwt, &new_proof));
        assert!(
            matches!(
                &first,
                GateDecision::Deny {
                    stage: Stage::Budget,
                    ..
                }
            ),
            "{first:?}"
        );
        assert_eq!(gate.prune_count(), 1, "the first call at capacity scans");

        // A second call at the same clock second must not scan again.
        let second = gate.evaluate(&call("merge", &new_args, &new_cwt, &new_proof));
        assert!(
            matches!(
                &second,
                GateDecision::Deny {
                    stage: Stage::Budget,
                    ..
                }
            ),
            "{second:?}"
        );
        assert_eq!(
            gate.prune_count(),
            1,
            "a repeat call within the same clock second must not re-scan"
        );

        // Once the clock passes the counters' expiry, the next call scans
        // again, frees them, and the new permit is admitted.
        CLOCK.store(NOW + 60, Ordering::SeqCst);
        let admitted = gate.evaluate(&call("merge", &new_args, &new_cwt, &new_proof));
        assert!(
            matches!(admitted, GateDecision::Allow { .. }),
            "{admitted:?}"
        );
        assert_eq!(
            gate.prune_count(),
            2,
            "expiry made the table worth scanning again"
        );
        assert_eq!(gate.usage_len(), 1, "every expired counter was freed");
    }

    /// A call the upstream never received must not cost its permit an
    /// invocation: the gate spends the budget before the dispatch, so the
    /// dispatcher hands it back when the dispatch fails.
    #[test]
    #[spec("PDG-006")]
    fn a_refund_returns_one_invocation_to_the_budget() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&issuer, &holder, "merge", &args, 1, NOW + 300, 7);
        let proof = prove_invocation(
            &holder,
            &permit_id(&cwt),
            "merge",
            &args,
            HashAlgorithm::Sha256,
        );

        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Allow { .. }
        ));
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Budget,
                ..
            }
        ));

        assert!(gate.refund(permit_id(&cwt)), "the counter exists to refund");
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Allow { .. },
        ));
    }

    /// A refund never invents budget: it cannot take a counter below zero,
    /// and it does not create one for a permit that has spent nothing.
    #[test]
    #[spec("PDG-006")]
    fn a_refund_never_creates_budget() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&issuer, &holder, "merge", &args, 1, NOW + 300, 7);
        let proof = prove_invocation(
            &holder,
            &permit_id(&cwt),
            "merge",
            &args,
            HashAlgorithm::Sha256,
        );

        // Nothing spent yet: nothing to give back, and no counter created.
        assert!(!gate.refund(permit_id(&cwt)));
        assert_eq!(gate.usage_len(), 0);

        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Allow { .. }
        ));
        assert!(gate.refund(permit_id(&cwt)));
        // A second refund has nothing left to return, and the permit still
        // has exactly the one invocation its budget allows.
        assert!(!gate.refund(permit_id(&cwt)));
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Allow { .. }
        ));
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Budget,
                ..
            }
        ));
    }

    /// `refund_invocation` verifies the permit rather than decoding it, so a
    /// refund can only be aimed at a permit the caller can present. A permit
    /// this gate does not trust refunds nothing.
    #[test]
    fn refund_invocation_verifies_the_permit_it_credits() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&issuer, &holder, "merge", &args, 1, NOW + 300, 7);
        let proof = prove_invocation(
            &holder,
            &permit_id(&cwt),
            "merge",
            &args,
            HashAlgorithm::Sha256,
        );
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Allow { .. }
        ));

        // A permit from an issuer this gate does not trust names an identity
        // it will not credit, even though the token decodes perfectly well.
        let rogue = PermitSigner::Ed25519(AgentKeypair::generate());
        let forged = permit(&rogue, &holder, "merge", &args, 1, NOW + 300, 7);
        assert!(!gate.refund_invocation(&forged));

        assert!(gate.refund_invocation(&cwt));
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Allow { .. }
        ));
    }

    /// Canonicalizing and hashing the arguments is the gate's only unbounded
    /// work, and it happens twice per call. Oversized arguments are refused
    /// before any of it.
    #[test]
    #[spec("PDG-010")]
    fn oversized_arguments_are_denied_at_policy() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let huge = json!({"blob": "a".repeat(MAX_ARGUMENTS_BYTES)});
        let cwt = permit(&issuer, &holder, "merge", &huge, 2, NOW + 300, 7);
        let proof = prove_invocation(
            &holder,
            &permit_id(&cwt),
            "merge",
            &huge,
            HashAlgorithm::Sha256,
        );

        let decision = gate.evaluate(&call("merge", &huge, &cwt, &proof));
        assert!(
            matches!(
                &decision,
                GateDecision::Deny {
                    stage: Stage::Policy,
                    reason,
                } if reason.contains("arguments")
            ),
            "{decision:?}"
        );
        assert_eq!(gate.usage_len(), 0, "a denied call consumes no budget");

        // The same permit with arguments under the cap is admitted, so the
        // refusal is about size and nothing else.
        let small = json!({"blob": "a"});
        let cwt = permit(&issuer, &holder, "merge", &small, 2, NOW + 300, 7);
        let proof = prove_invocation(
            &holder,
            &permit_id(&cwt),
            "merge",
            &small,
            HashAlgorithm::Sha256,
        );
        assert!(matches!(
            gate.evaluate(&call("merge", &small, &cwt, &proof)),
            GateDecision::Allow { .. }
        ));
    }

    #[test]
    fn evaluation_stays_under_the_gate_budget() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&issuer, &holder, "merge", &args, 10_000, NOW + 300, 7);
        let proof = prove_invocation(
            &holder,
            &permit_id(&cwt),
            "merge",
            &args,
            HashAlgorithm::Sha256,
        );
        let mut samples: Vec<u128> = (0..200)
            .map(|_| {
                let start = std::time::Instant::now();
                let _ = gate.evaluate(&call("merge", &args, &cwt, &proof));
                start.elapsed().as_micros()
            })
            .collect();
        samples.sort_unstable();
        let p95 = samples[189];
        assert!(p95 < 25_000, "p95 {p95}µs exceeds the 25ms gate budget");
    }
}
