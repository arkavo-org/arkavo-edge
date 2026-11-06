//! Shared test harness: local trust records, signers, and a verifier over a temporary
//! SQLite replay store with a manual clock.

use arkavo_tap::*;
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;

pub const NOW: i64 = 1_893_456_000;
pub const MERCHANT: &str = "http://127.0.0.1:3001";

pub fn authority() -> TrustAuthority {
    TrustAuthority {
        name: "arkavo-local-demo".into(),
        environment: Environment::Local,
    }
}

pub fn origin() -> ExternalOrigin {
    ExternalOrigin::parse(MERCHANT).unwrap()
}

/// A signer and the record a registry would hold for it.
pub fn agent(seed: u8, key_id: &str, status: KeyStatus) -> (Ed25519Signer, KeyRecord) {
    let signer = Ed25519Signer::new(
        &[seed; 32],
        key_id,
        SigningPolicy {
            destinations: vec![MERCHANT.into(), "https://example.com".into()],
            operations: vec![Operation::Read, Operation::Checkout, Operation::Commit],
        },
    );
    let record = KeyRecord::ed25519(KeyFields {
        authority: authority(),
        key_id: key_id.into(),
        binding: ActorBinding {
            provider: "arkavo".into(),
            runtime: Some(format!("runtime-{seed}")),
        },
        public_key: &signer.public_key(),
        status,
        not_before: NOW - 3_600,
        not_after: NOW + 86_400,
        purposes: vec![Purpose::Browse, Purpose::Pay],
        retrieved_at: NOW,
        refresh_by: NOW + 60,
        provenance: "static test registry".into(),
    })
    .unwrap();
    (signer, record)
}

/// Resolves from a fixed set of records for one authority.
pub struct StaticResolver {
    pub authority: TrustAuthority,
    pub records: HashMap<String, KeyRecord>,
    pub unavailable: bool,
}

impl StaticResolver {
    pub fn new(records: Vec<KeyRecord>) -> Self {
        StaticResolver {
            authority: authority(),
            records: records.into_iter().map(|r| (r.key_id.clone(), r)).collect(),
            unavailable: false,
        }
    }
}

impl TrustedKeyResolver for StaticResolver {
    fn resolve(
        &self,
        authority: &TrustAuthority,
        key_id: &str,
    ) -> impl Future<Output = Result<KeyRecord, ResolveError>> + Send {
        let answer = if self.unavailable {
            Err(ResolveError::Unavailable("registry down".into()))
        } else if *authority != self.authority {
            Err(ResolveError::Unknown)
        } else {
            self.records
                .get(key_id)
                .cloned()
                .ok_or(ResolveError::Unknown)
        };
        std::future::ready(answer)
    }
}

pub type Verifier = TapVerifier<StaticResolver, SqliteReplayStore<ManualClock>, ManualClock>;

pub async fn verifier_at(
    replay: &Path,
    resolver: StaticResolver,
    clock: ManualClock,
    authority: TrustAuthority,
) -> Verifier {
    let store = SqliteReplayStore::open(replay, clock.clone(), 30)
        .await
        .unwrap();
    TapVerifier::new(
        VerifierConfig {
            authority,
            audience: origin(),
            max_header_bytes: 16 * 1024,
        },
        resolver,
        store,
        clock,
    )
}

pub fn request(method: &str, target: &str, headers: &[(&str, &str)]) -> RequestContext {
    RequestContext::new(
        origin(),
        method,
        target,
        headers
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect(),
    )
    .unwrap()
}

/// A write request with its content digest and idempotency key.
pub fn write(target: &str, body: &[u8], idempotency_key: &str) -> RequestContext {
    let digest = arkavo_tap::digest::content_digest(body);
    request(
        "POST",
        target,
        &[
            ("content-type", "application/json"),
            ("content-digest", &digest),
            ("idempotency-key", idempotency_key),
        ],
    )
}

/// `ctx` with the signature headers added.
pub fn with_signature(ctx: &RequestContext, headers: &SignatureHeaders) -> RequestContext {
    with_fields(ctx, &headers.signature_input, &headers.signature)
}

pub fn with_fields(ctx: &RequestContext, input: &str, signature: &str) -> RequestContext {
    let mut fields: Vec<(String, String)> = Vec::new();
    for name in ["content-type", "content-digest", "idempotency-key"] {
        if let Some(v) = ctx.header(name) {
            fields.push((name.into(), v));
        }
    }
    fields.push(("signature-input".into(), input.into()));
    fields.push(("signature".into(), signature.into()));
    let target = ctx.target_uri();
    let path = &target[ctx.origin().as_origin().len()..];
    RequestContext::new(ctx.origin().clone(), ctx.method(), path, fields).unwrap()
}

/// Signs `ctx` under the Arkavo profile.
pub fn sign(
    signer: &Ed25519Signer,
    ctx: &RequestContext,
    operation: Operation,
    purpose: Purpose,
    created: i64,
) -> RequestContext {
    let op = arkavo_tap::signer::prepare(
        ctx.clone(),
        Profile::ArkavoRfc9421V1,
        operation,
        purpose,
        signer.key_id(),
        created,
        120,
    )
    .unwrap();
    let headers = arkavo_tap::signer::sign(signer, &op).unwrap();
    with_signature(ctx, &headers)
}

pub fn route(operation: Operation, purpose: Purpose) -> RoutePolicy {
    RoutePolicy {
        profile: Profile::ArkavoRfc9421V1,
        operation,
        purposes: vec![purpose],
    }
}
