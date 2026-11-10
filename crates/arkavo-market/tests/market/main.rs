//! A market that checks requests the way the real one does, an identity
//! that mints, and the client wired to both.
//!
//! The stand-in market rebuilds the signed text from the request it received
//! (method, path, raw body, bearer text) following the published contract,
//! so a client that signs anything other than the bytes it sends is refused
//! here as it would be in production.

// `#[tokio::test]` expands to `Runtime::block_on`, which `.clippy.toml`
// disallows in the product, where nesting runtimes can panic.
#![allow(clippy::disallowed_methods)]

mod requests;
mod tool;

use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use arkavo_agent_auth::AgentAuthConfig;
use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use arkavo_market::{MarketClient, REQUEST_ENTITLEMENT};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const OFFER: &str = "arkavo-online-play";

/// Identity: a challenge, then a token that differs on every mint.
struct Minting {
    entitlements: Vec<String>,
    minted: AtomicUsize,
}

impl Respond for Minting {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let n = self.minted.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200).set_body_json(json!({
            "token": format!("token-{n}"),
            "expires_at": chrono::Utc::now().timestamp() + 900,
            "entitlements": self.entitlements,
        }))
    }
}

async fn identity(entitlements: &[&str]) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/agents/challenge"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "challenge": STANDARD.encode([9u8; 32]),
            "nonce": "nonce-1",
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/agents/token"))
        .respond_with(Minting {
            entitlements: entitlements.iter().map(ToString::to_string).collect(),
            minted: AtomicUsize::new(0),
        })
        .mount(&server)
        .await;
    server
}

/// The market's checks on a request, then the next scripted answer.
struct Market {
    key: AgentPublicKey,
    answers: Mutex<VecDeque<ResponseTemplate>>,
    nonces: Mutex<HashSet<String>>,
}

impl Market {
    fn new(key: AgentPublicKey, answers: Vec<ResponseTemplate>) -> Self {
        Self {
            key,
            answers: Mutex::new(answers.into()),
            nonces: Mutex::new(HashSet::new()),
        }
    }

    fn check(&self, request: &Request) -> Result<(), &'static str> {
        let header = |name: &str| request.headers.get(name).and_then(|v| v.to_str().ok());
        if header("origin").is_some() {
            return Err("an Origin header marks a browser");
        }
        let token = header("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or("no bearer token")?;
        let proof = header("arkavo-signature").ok_or("no signature header")?;
        if proof.len() > 512 {
            return Err("signature header too long");
        }
        let mut parts = proof.split(", ");
        let (Some(t), Some(n), Some(s), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err("signature header is not three parts");
        };
        let t = t.strip_prefix("t=").ok_or("no t")?;
        let n = n.strip_prefix("n=").ok_or("no n")?;
        let s = s.strip_prefix("s=").ok_or("no s")?;
        if t.starts_with('0') || !t.bytes().all(|b| b.is_ascii_digit()) {
            return Err("t is not a plain decimal");
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        if now.abs_diff(t.parse().map_err(|_| "t does not parse")?) > 60 {
            return Err("t is outside 60 seconds");
        }
        if URL_SAFE_NO_PAD
            .decode(n)
            .map_err(|_| "n is not base64url")?
            .len()
            != 16
        {
            return Err("n is not 16 bytes");
        }
        let signature = URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|_| "s is not base64url")?;
        let sha256 = |bytes: &[u8]| URL_SAFE_NO_PAD.encode(Sha256::digest(bytes));
        let path_and_query = match request.url.query() {
            Some(query) => format!("{}?{query}", request.url.path()),
            None => request.url.path().to_string(),
        };
        let signed = format!(
            "arkavo-market-request/1\n{}\n{path_and_query}\n{t}\n{n}\n{}\n{}",
            request.method.as_str(),
            sha256(&request.body),
            sha256(token.as_bytes()),
        );
        self.key
            .verify(signed.as_bytes(), &signature)
            .map_err(|_| "signature does not verify")?;
        if !self.nonces.lock().unwrap().insert(n.to_string()) {
            return Err("nonce reused");
        }
        Ok(())
    }
}

impl Respond for Market {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        if let Err(reason) = self.check(request) {
            return ResponseTemplate::new(401)
                .insert_header("x-refused-because", reason)
                .set_body_json(json!({"status": "unauthorized"}));
        }
        let mut answers = self.answers.lock().unwrap();
        if answers.len() > 1 {
            answers.pop_front().unwrap()
        } else {
            answers.front().cloned().expect("an answer is scripted")
        }
    }
}

fn open_request(term: u8) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "request": "rq_AAAAAAAAAAAAAAAAAAAAAA",
        "url": "https://buy.stripe.com/test_abc",
        "term": term,
        "status": "open",
    }))
}

fn refusal(status: u16, word: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({ "status": word }))
}

struct World {
    identity: MockServer,
    market: MockServer,
    keypair: AgentKeypair,
    client: MarketClient,
}

async fn world_with(entitlements: &[&str], answers: Vec<ResponseTemplate>) -> World {
    let keypair = AgentKeypair::generate();
    let identity = identity(entitlements).await;
    let market = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(Market::new(keypair.public_key(), answers))
        .mount(&market)
        .await;
    let client = MarketClient::with_endpoints(
        &market.uri(),
        AgentAuthConfig::new(identity.uri()).with_max_retries(0),
    )
    .unwrap();
    World {
        identity,
        market,
        keypair,
        client,
    }
}

async fn world(answers: Vec<ResponseTemplate>) -> World {
    world_with(&[REQUEST_ENTITLEMENT], answers).await
}

async fn market_requests(world: &World) -> Vec<Request> {
    world.market.received_requests().await.unwrap()
}
