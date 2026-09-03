//! The published COSE key set and its cache.
//!
//! `/.well-known/cose-keys` serves a CBOR array of COSE_Key maps (not JWKS
//! JSON). Each entry carries the raw 32-byte RFC 7638 thumbprint as its `kid`,
//! which is the same value the CWT's protected header names; matching is plain
//! byte equality, never a recomputed thumbprint.

use crate::{Claims, CwtError, VerifyOptions, verify::verify};
use coset::{CborSerializable, CoseKeySet};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

/// The largest `/.well-known/cose-keys` body this crate will read.
///
/// A key set is a handful of 32-byte coordinates; anything past this is a
/// misbehaving or hostile endpoint, and reading it to the end would be its
/// denial of service.
pub const MAX_KEY_SET_BYTES: usize = 64 * 1024;

/// How long a fetch of the key set may take, connection and body together.
///
/// [`CachedKeySet`] holds its refresh lock across this call, so every
/// verification waiting on the refresh waits on this too: without a bound,
/// an endpoint that accepts a connection and never answers would stall them
/// all for as long as it cared to. Ten seconds is far above a healthy fetch
/// of a few hundred bytes and far below anything a caller would sit through.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// The ES256 verification keys the issuer currently publishes.
pub struct KeySet {
    keys: Vec<(Vec<u8>, crate::VerifyingKey)>,
}

impl KeySet {
    /// Parse a `/.well-known/cose-keys` body.
    ///
    /// Entries that are not EC2 P-256 keys, or that carry no `kid`, are skipped
    /// rather than rejected: the issuer may publish keys for algorithms this
    /// verifier does not handle, and one such entry must not blind it to the
    /// ES256 keys alongside it.
    ///
    /// The body is as untrusted as a token is — it arrives over the network
    /// from an endpoint this crate does not control — and reaches the same
    /// recursive decoder, so the same nesting bound is applied to it first.
    pub fn from_cbor(bytes: &[u8]) -> Result<Self, CwtError> {
        // The depth check speaks about COSE_Sign1s, so its reason is
        // re-labelled rather than restated: there is now more than one way it
        // can refuse, and reporting them all as a depth breach would be
        // telling the operator the wrong thing about their endpoint.
        crate::depth::check(bytes).map_err(|error| match error {
            CwtError::Cose(reason) => CwtError::KeySet(reason),
            other => CwtError::KeySet(other.to_string()),
        })?;
        let set = CoseKeySet::from_slice(bytes).map_err(|e| CwtError::KeySet(e.to_string()))?;
        let mut keys = Vec::with_capacity(set.0.len());
        for key in &set.0 {
            if key.key_id.is_empty() {
                continue;
            }
            if let Ok(crate::VerifyingKey::P256(vk)) = crate::VerifyingKey::from_cose_key(key) {
                keys.push((key.key_id.clone(), crate::VerifyingKey::P256(vk)));
            }
        }
        if keys.is_empty() {
            return Err(CwtError::KeySet("no usable ES256 P-256 keys".into()));
        }
        Ok(Self { keys })
    }

    /// Fetch and parse the key set published at `url`, bounded by
    /// [`FETCH_TIMEOUT`].
    ///
    /// The body is read chunk by chunk and refused as soon as it passes
    /// [`MAX_KEY_SET_BYTES`], so an endpoint that streams without end — or a
    /// redirect to one — cannot make this allocate without bound. The
    /// advertised `Content-Length` is not consulted: it is the endpoint's own
    /// claim, and the accumulated length is the bound that actually holds.
    pub async fn fetch(url: &str) -> Result<Self, CwtError> {
        Self::fetch_with_timeout(url, FETCH_TIMEOUT).await
    }

    /// [`Self::fetch`] with the timeout named explicitly, which is how a test
    /// reaches it without waiting out the real one.
    pub async fn fetch_with_timeout(url: &str, timeout: Duration) -> Result<Self, CwtError> {
        // The timeout covers the connection and the whole exchange, the body
        // read included: a server that accepts, answers a header and then
        // trickles is the same denial of service as one that never answers.
        // It is set per request rather than on the client, because the client
        // outlives any one fetch.
        let mut response = client()?
            .get(url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| CwtError::Fetch(e.to_string()))?
            .error_for_status()
            .map_err(|e| CwtError::Fetch(e.to_string()))?;
        let mut body: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| CwtError::Fetch(e.to_string()))?
        {
            if body.len() + chunk.len() > MAX_KEY_SET_BYTES {
                return Err(CwtError::KeySet("key set larger than 64 KiB".into()));
            }
            body.extend_from_slice(&chunk);
        }
        Self::from_cbor(&body)
    }

    pub(crate) fn get(&self, kid: &[u8]) -> Option<&crate::VerifyingKey> {
        self.keys
            .iter()
            .find(|(candidate, _)| candidate == kid)
            .map(|(_, key)| key)
    }
}

/// The HTTP client every key-set fetch goes through, built once.
///
/// A `reqwest::Client` owns a connection pool and a resolver; building one
/// per fetch throws both away, so a verifier that refreshes on a schedule
/// paid for a fresh TLS handshake every time and kept nothing warm. It
/// carries no timeout of its own — [`KeySet::fetch_with_timeout`] sets that
/// per request, since the bound is the caller's and the client is shared.
///
/// A builder that cannot produce a client is a fault of this process, not of
/// the endpoint, and it is the same fault on every call, so the failure is
/// remembered rather than retried into.
///
/// The client is process-wide, not per-verifier and not per-runtime: every
/// [`CachedKeySet`] in the process shares this one, and so does every tokio
/// runtime in it, including the one a `#[tokio::test]` builds and drops
/// around each test. Sharing a pooled connection across runtimes is the one
/// thing that would matter — a socket parked in the pool is registered with
/// the reactor of the runtime that opened it, and a later runtime reusing it
/// would find that reactor gone. Nothing here can reach that state: every
/// test serves its key set from a `wiremock` server bound to a fresh
/// ephemeral port, so no later request ever matches a pooled connection, and
/// in production the runtime outlives the process's fetches.
fn client() -> Result<&'static reqwest::Client, CwtError> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| CwtError::Fetch(e.clone()))
}

struct Cached {
    keys: Arc<KeySet>,
    fetched_at: Instant,
}

/// A [`KeySet`] that re-fetches when a token names a `kid` it does not hold.
///
/// Refetching happens no more than once per `ttl`, so a token signed by a key
/// that will never be published cannot turn into a fetch per verification.
/// That holds under concurrency too: fetches are serialized on `refreshing`,
/// and whoever waits there re-reads the cache before deciding, so a burst of
/// unknown-kid verifications collapses into one request rather than one each.
pub struct CachedKeySet {
    url: String,
    ttl: Duration,
    cached: RwLock<Option<Cached>>,
    /// Held across the fetch so concurrent refreshes coalesce.
    refreshing: tokio::sync::Mutex<()>,
}

impl CachedKeySet {
    pub fn new(url: impl Into<String>, ttl: Duration) -> Self {
        Self {
            url: url.into(),
            ttl,
            cached: RwLock::new(None),
            refreshing: tokio::sync::Mutex::new(()),
        }
    }

    /// Verify `token` against the cached key set, refreshing once if the token's
    /// `kid` is unknown and the cache is at least `ttl` old.
    pub async fn verify(&self, token: &str, opts: &VerifyOptions<'_>) -> Result<Claims, CwtError> {
        let current = self.snapshot();
        let (keys, fetched_at) = match current {
            Some(cached) => cached,
            None => (self.refresh().await?, Instant::now()),
        };

        match verify(token, &keys, opts) {
            Err(CwtError::UnknownKid(kid)) => {
                if fetched_at.elapsed() < self.ttl {
                    return Err(CwtError::UnknownKid(kid));
                }
                let refreshed = self.refresh().await?;
                verify(token, &refreshed, opts)
            }
            other => other,
        }
    }

    fn snapshot(&self) -> Option<(Arc<KeySet>, Instant)> {
        let guard = self
            .cached
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .as_ref()
            .map(|cached| (cached.keys.clone(), cached.fetched_at))
    }

    /// Fetch the key set, unless a concurrent caller just did.
    ///
    /// The `refreshing` lock is what makes "at most once per ttl" hold when
    /// several verifications miss at the same time: the first fetches, the
    /// rest wait, and each of them then finds a cache younger than the ttl
    /// and takes it instead of issuing a request of its own.
    async fn refresh(&self) -> Result<Arc<KeySet>, CwtError> {
        let guard = self.refreshing.lock().await;
        if let Some((keys, fetched_at)) = self.snapshot()
            && fetched_at.elapsed() < self.ttl
        {
            return Ok(keys);
        }
        let keys = Arc::new(KeySet::fetch(&self.url).await?);
        let mut cached = self
            .cached
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *cached = Some(Cached {
            keys: keys.clone(),
            fetched_at: Instant::now(),
        });
        drop(cached);
        drop(guard);
        Ok(keys)
    }
}

#[cfg(test)]
// The `#[tokio::test]` macro expands to `Runtime::block_on`, which
// `.clippy.toml` disallows outside test code.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use coset::CoseKeyBuilder;
    use p256::ecdsa::SigningKey;

    /// A structurally valid COSE_Key whose `x` (or `y`) coordinate is 31 bytes
    /// — the shape a leading-zero-stripped coordinate produces — must be
    /// rejected through the crate's public parse path, not panic the process.
    /// `generic-array`'s `From<&[T]>` panics on this input; the fix routes the
    /// length check through `CwtError::KeySet` instead.
    fn cose_key_with(x: Vec<u8>, y: Vec<u8>) -> Vec<u8> {
        let key = CoseKeyBuilder::new_ec2_pub_key(coset::iana::EllipticCurve::P_256, x, y)
            .algorithm(coset::iana::Algorithm::ES256)
            .key_id(vec![0xAA; 32])
            .build();
        CoseKeySet(vec![key]).to_vec().expect("encode key set")
    }

    fn valid_coordinates() -> (Vec<u8>, Vec<u8>) {
        let vk = *SigningKey::random(&mut rand::rngs::OsRng).verifying_key();
        let point = vk.to_encoded_point(false);
        (
            point.x().expect("x coordinate").to_vec(),
            point.y().expect("y coordinate").to_vec(),
        )
    }

    #[test]
    fn from_cbor_rejects_31_byte_x_coordinate_without_panicking() {
        let (x, y) = valid_coordinates();
        let bytes = cose_key_with(x[..31].to_vec(), y);

        match KeySet::from_cbor(&bytes) {
            Err(CwtError::KeySet(_)) => {}
            Err(other) => panic!("expected CwtError::KeySet, got {other:?}"),
            Ok(_) => panic!("31-byte x must be rejected, not accepted"),
        }
    }

    /// A key endpoint that answers with megabytes — compromised, misbehaving,
    /// or a redirect to something else entirely — must not be read to the
    /// end. The body is refused as it arrives, at 64 KiB.
    #[tokio::test]
    async fn fetch_refuses_an_oversized_key_set() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/cose-keys"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(vec![0u8; MAX_KEY_SET_BYTES + 1]),
            )
            .mount(&server)
            .await;

        let url = format!("{}/.well-known/cose-keys", server.uri());
        match KeySet::fetch(&url).await {
            Err(CwtError::KeySet(message)) => {
                assert!(message.contains("64 KiB"), "message: {message}");
            }
            Err(other) => panic!("expected a size refusal, got {other:?}"),
            Ok(_) => panic!("an oversized body must not be parsed"),
        }
    }

    /// An endpoint that accepts the connection and then says nothing must not
    /// hold the fetch open: the refresh lock is held across it, so every
    /// verification waiting behind it would wait exactly as long.
    #[tokio::test]
    async fn fetch_gives_up_on_an_endpoint_that_does_not_answer() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/cose-keys"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(vec![0u8; 8])
                    .set_delay(Duration::from_secs(30)),
            )
            .mount(&server)
            .await;

        let url = format!("{}/.well-known/cose-keys", server.uri());
        let started = Instant::now();
        match KeySet::fetch_with_timeout(&url, Duration::from_millis(100)).await {
            Err(CwtError::Fetch(_)) => {}
            Err(other) => panic!("expected a fetch failure, got {other:?}"),
            Ok(_) => panic!("a body that never arrives must not parse"),
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the fetch must give up on its own timeout, not the server's"
        );
    }

    /// The cap refuses what is too large without refusing what is not: a
    /// real key set, served by the same path, still parses.
    #[tokio::test]
    async fn fetch_accepts_a_key_set_under_the_cap() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let (x, y) = valid_coordinates();
        let body = cose_key_with(x, y);
        assert!(body.len() < MAX_KEY_SET_BYTES);

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/cose-keys"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;

        let url = format!("{}/.well-known/cose-keys", server.uri());
        let keys = KeySet::fetch(&url).await.expect("key set under the cap");
        assert!(keys.get(&[0xAA; 32]).is_some());
    }

    /// A key set is decoded by the same recursive decoder a token is, and it
    /// comes from the same place: the network. Nesting that would exhaust a
    /// worker's stack has to be refused before `CoseKeySet::from_slice` walks
    /// it, not after.
    #[test]
    fn from_cbor_refuses_a_deeply_nested_key_set() {
        let mut deep = vec![0x81u8; 200];
        deep.push(0x00);
        assert!(deep.len() < MAX_KEY_SET_BYTES);

        match KeySet::from_cbor(&deep) {
            Err(CwtError::KeySet(message)) => {
                assert!(message.contains("nesting depth"), "message: {message}");
            }
            Err(other) => panic!("expected a depth refusal, got {other:?}"),
            Ok(_) => panic!("a deeply nested key set must not be parsed"),
        }

        // A real key set is unaffected: it is two levels deep and parses.
        let (x, y) = valid_coordinates();
        assert!(KeySet::from_cbor(&cose_key_with(x, y)).is_ok());
    }

    #[test]
    fn from_cbor_rejects_31_byte_y_coordinate_without_panicking() {
        let (x, y) = valid_coordinates();
        let bytes = cose_key_with(x, y[..31].to_vec());

        match KeySet::from_cbor(&bytes) {
            Err(CwtError::KeySet(_)) => {}
            Err(other) => panic!("expected CwtError::KeySet, got {other:?}"),
            Ok(_) => panic!("31-byte y must be rejected, not accepted"),
        }
    }
}
