//! The verification sequence for a protected request.
//!
//! Order matters: cheap structural checks first, cryptography after the limits, and the
//! nonce claim last, after the signature and digest verify, so a forged request can never
//! consume a real agent's nonce. Every decision, accepted or rejected, produces a record.

use crate::base::signature_base;
use crate::clock::Clock;
use crate::context::{ExternalOrigin, RequestContext};
use crate::digest;
use crate::error::{RejectKind, Rejection};
use crate::evidence::{DecisionRecord, RequestBinding, VerifiedAgent};
use crate::fields::{self, ParsedSignature};
use crate::keys::{
    KeyRecord, KeyStatus, ResolveError, TrustAuthority, TrustedKeyResolver, key_id_is_valid,
};
use crate::profile::{Operation, Profile, Purpose};
use crate::replay::{ClaimOutcome, ReplayScope, ReplayStore};
use std::time::Instant;

/// What a route requires, from server configuration. Never taken from the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePolicy {
    pub profile: Profile,
    pub operation: Operation,
    /// Purposes the route accepts.
    pub purposes: Vec<Purpose>,
}

/// The verifier's fixed settings.
#[derive(Debug, Clone)]
pub struct VerifierConfig {
    /// The one trust authority keys are resolved through.
    pub authority: TrustAuthority,
    /// This merchant's external origin: the audience signatures must address.
    pub audience: ExternalOrigin,
    pub max_header_bytes: usize,
}

/// Verifies TAP signatures for one merchant audience.
pub struct TapVerifier<R, S, C> {
    config: VerifierConfig,
    resolver: R,
    replay: S,
    clock: C,
}

impl<R: TrustedKeyResolver, S: ReplayStore, C: Clock> TapVerifier<R, S, C> {
    pub fn new(config: VerifierConfig, resolver: R, replay: S, clock: C) -> Self {
        TapVerifier {
            config,
            resolver,
            replay,
            clock,
        }
    }

    pub fn config(&self) -> &VerifierConfig {
        &self.config
    }

    /// Verifies `ctx` (with its bounded `content`) against `route`, and emits the decision
    /// record under `decision_id`.
    pub async fn verify(
        &self,
        ctx: &RequestContext,
        content: &[u8],
        route: &RoutePolicy,
        decision_id: &str,
    ) -> Result<VerifiedAgent, Rejection> {
        let started = Instant::now();
        let mut key: Option<(String, String)> = None;
        let result = self.run(ctx, content, route, decision_id, &mut key).await;
        let (key_id, key_thumbprint) = key.unzip();
        let record = DecisionRecord {
            decision_id: decision_id.to_owned(),
            profile: route.profile,
            accepted: result.is_ok(),
            result: match &result {
                Ok(_) => "accepted".to_owned(),
                Err(r) => r.kind.code().to_owned(),
            },
            reason: result
                .as_ref()
                .err()
                .map(|r| r.detail.clone())
                .unwrap_or_default(),
            key_id,
            key_thumbprint,
            authority: self.config.authority.to_string(),
            elapsed_micros: started.elapsed().as_micros() as u64,
        };
        record.emit();
        result
    }

    async fn run(
        &self,
        ctx: &RequestContext,
        content: &[u8],
        route: &RoutePolicy,
        decision_id: &str,
        key_seen: &mut Option<(String, String)>,
    ) -> Result<VerifiedAgent, Rejection> {
        if ctx.header_bytes() > self.config.max_header_bytes {
            return Err(Rejection::new(
                RejectKind::HeadersTooLarge,
                "header fields exceed the limit",
            ));
        }
        if *ctx.origin() != self.config.audience {
            return Err(Rejection::new(
                RejectKind::InvalidSignature,
                "the request is not addressed to this merchant",
            ));
        }
        let parsed = parse_fields(ctx)?;
        let profile = route.profile;
        check_coverage(&parsed, profile, route.operation)?;
        let purpose = check_params(&parsed, profile, route)?;
        self.check_time(&parsed, profile)?;

        let record = self.resolve(&parsed.params.keyid).await?;
        *key_seen = Some((record.key_id.clone(), record.thumbprint()));
        self.check_key(&record, purpose)?;

        let base = signature_base(ctx, &parsed.components, &parsed.signature_params)?;
        record.verify(&base, &parsed.signature)?;
        let content_digest = if parsed.components.iter().any(|c| c == "content-digest") {
            let field = ctx.component("content-digest")?;
            digest::verify(&field, content)?;
            Some(field)
        } else {
            None
        };
        let idempotency_key = if parsed.components.iter().any(|c| c == "idempotency-key") {
            Some(ctx.component("idempotency-key")?)
        } else {
            None
        };

        let scope = ReplayScope {
            authority: self.config.authority.to_string(),
            key_thumbprint: record.thumbprint(),
            audience: self.config.audience.as_origin(),
            nonce: parsed.params.nonce.clone(),
        };
        match self
            .replay
            .claim(&scope, parsed.params.expires, decision_id)
            .await
        {
            Ok(ClaimOutcome::Claimed) => {}
            Ok(ClaimOutcome::AlreadyUsed) => {
                return Err(Rejection::new(RejectKind::Replay, "nonce already used"));
            }
            Err(e) => return Err(Rejection::new(RejectKind::Unavailable, e)),
        }

        Ok(VerifiedAgent {
            authority: record.authority.clone(),
            key_id: record.key_id.clone(),
            key_thumbprint: record.thumbprint(),
            provider: record.binding.provider.clone(),
            runtime: record.binding.runtime.clone(),
            profile,
            operation: route.operation,
            purpose,
            covered: parsed.components,
            binding: RequestBinding {
                method: ctx.method().to_owned(),
                target_uri: ctx.target_uri(),
                content_digest,
                idempotency_key,
            },
            nonce: parsed.params.nonce,
            created: parsed.params.created,
            expires: parsed.params.expires,
            decision_id: decision_id.to_owned(),
            provenance: record.provenance.clone(),
        })
    }

    fn check_time(&self, parsed: &ParsedSignature, profile: Profile) -> Result<(), Rejection> {
        let now = self.clock.now();
        let p = &parsed.params;
        let expired = |why: &str| Err(Rejection::new(RejectKind::Expired, why.to_owned()));
        if p.created > now + profile.future_tolerance() {
            return expired("created is in the future");
        }
        if p.expires <= p.created {
            return expired("expires is not after created");
        }
        if p.expires - p.created > profile.max_lifetime() {
            return expired("the signature lifetime exceeds the profile's maximum");
        }
        if now >= p.expires {
            return expired("the signature has expired");
        }
        Ok(())
    }

    async fn resolve(&self, key_id: &str) -> Result<KeyRecord, Rejection> {
        if !key_id_is_valid(key_id) {
            return Err(Rejection::malformed("keyid is not a valid key identifier"));
        }
        self.resolver
            .resolve(&self.config.authority, key_id)
            .await
            .map_err(|e| match e {
                ResolveError::Unknown => Rejection::new(
                    RejectKind::UntrustedActor,
                    "the trust authority has no such key",
                ),
                ResolveError::Unavailable(why) => Rejection::new(RejectKind::Unavailable, why),
            })
    }

    fn check_key(&self, record: &KeyRecord, purpose: Purpose) -> Result<(), Rejection> {
        let now = self.clock.now();
        let untrusted = |why: &str| Err(Rejection::new(RejectKind::UntrustedActor, why.to_owned()));
        if record.status == KeyStatus::Revoked {
            return untrusted("the key is revoked");
        }
        if now < record.not_before || now >= record.not_after {
            return untrusted("the key is outside its validity period");
        }
        if !record.purposes.contains(&purpose) {
            return Err(Rejection::new(
                RejectKind::PurposeDenied,
                "the key may not declare this purpose",
            ));
        }
        Ok(())
    }
}

fn parse_fields(ctx: &RequestContext) -> Result<ParsedSignature, Rejection> {
    match (ctx.header("signature-input"), ctx.header("signature")) {
        (None, None) => Err(Rejection::new(
            RejectKind::MissingProof,
            "the route requires a TAP signature",
        )),
        (Some(input), Some(sig)) => fields::parse(&input, &sig),
        _ => Err(Rejection::malformed(
            "Signature-Input and Signature must be sent together",
        )),
    }
}

fn check_coverage(
    parsed: &ParsedSignature,
    profile: Profile,
    operation: Operation,
) -> Result<(), Rejection> {
    for needed in profile.required_components(operation) {
        if !parsed.components.iter().any(|c| c == needed) {
            return Err(Rejection::new(
                RejectKind::MissingProof,
                format!("{profile} requires {needed} to be covered for this operation"),
            ));
        }
    }
    Ok(())
}

fn check_params(
    parsed: &ParsedSignature,
    profile: Profile,
    route: &RoutePolicy,
) -> Result<Purpose, Rejection> {
    let p = &parsed.params;
    if p.alg != profile.algorithm() {
        return Err(Rejection::new(
            RejectKind::UntrustedActor,
            format!("alg {:?} is not accepted under {profile}", p.alg),
        ));
    }
    if !profile.nonce_is_valid(&p.nonce) {
        return Err(Rejection::malformed(
            "the nonce does not have the profile's form",
        ));
    }
    let purpose = Purpose::from_tag(&p.tag).ok_or_else(|| {
        Rejection::new(
            RejectKind::PurposeDenied,
            format!("tag {:?} is not a TAP purpose", p.tag),
        )
    })?;
    if !route.purposes.contains(&purpose) {
        return Err(Rejection::new(
            RejectKind::PurposeDenied,
            format!("the route does not accept {}", p.tag),
        ));
    }
    Ok(purpose)
}
