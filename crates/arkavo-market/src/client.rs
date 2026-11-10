//! The market's request API: asking for a term, and withdrawing the ask.

use std::net::IpAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use arkavo_agent_auth::{AgentAuthClient, AgentAuthConfig};
use arkavo_crypto::AgentKeypair;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, DATE, RETRY_AFTER};
use reqwest::{Method, Response, StatusCode};
use serde::{Deserialize, Serialize};

use crate::MarketError;

mod proof;

/// The market.
pub const MARKET_URL: &str = "https://market.arkavo.com";

/// What the owner delegates to let an agent ask. Identity matches the name
/// byte for byte, and the market looks for exactly this in the token.
pub const REQUEST_ENTITLEMENT: &str = "https://arkavo.net/attr/market/value/request";

const REQUESTS_PATH: &str = "/api/requests";

/// The market caps a request body at 1 KiB; an offer ID is far shorter.
const MAX_OFFER_BYTES: usize = 128;
const MAX_REQUEST_ID_BYTES: usize = 128;

/// The longest the market asks a caller to wait for an account's link slot.
const MAX_RETRY_AFTER_SECS: u64 = 30;

/// How far a request's time may be from the market's clock.
const CLOCK_TOLERANCE_SECS: i64 = 60;

/// How many months a prepaid term covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Term {
    Three,
    Twelve,
}

impl Term {
    pub fn months(self) -> u8 {
        match self {
            Self::Three => 3,
            Self::Twelve => 12,
        }
    }
}

impl TryFrom<u64> for Term {
    type Error = MarketError;

    fn try_from(months: u64) -> Result<Self, Self::Error> {
        match months {
            3 => Ok(Self::Three),
            12 => Ok(Self::Twelve),
            other => Err(MarketError::InvalidRequest(format!(
                "A term is 3 or 12 months, not {other}."
            ))),
        }
    }
}

/// An open request: `url` is the payment link to show the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketRequest {
    pub request: String,
    pub url: String,
    pub term: u8,
    pub status: String,
}

/// What a withdrawal found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Withdrawal {
    /// The request was open; its link no longer works.
    Withdrawn,
    /// The account has no open request with that ID: it was already paid,
    /// withdrawn or replaced.
    NotOpen,
}

#[derive(Serialize)]
struct Ask<'a> {
    offer: &'a str,
    term: u8,
}

/// Asks the market on the owner's behalf, as the agent whose key it is given.
pub struct MarketClient {
    origin: String,
    http: reqwest::Client,
    identity: AgentAuthClient,
}

impl MarketClient {
    /// A client for the market and identity in production.
    pub fn new() -> Result<Self, MarketError> {
        Self::with_endpoints(MARKET_URL, AgentAuthConfig::default())
    }

    /// A client for another market and identity, for a local market.
    ///
    /// The agent's token is a bearer token the platform accepts too, so it is
    /// sent only over TLS or to this machine, and only to the origin named
    /// here: no path, and no redirect is followed.
    pub fn with_endpoints(
        market_url: &str,
        identity: AgentAuthConfig,
    ) -> Result<Self, MarketError> {
        let origin = market_origin(market_url)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            origin,
            http,
            identity: AgentAuthClient::with_config(identity)?,
        })
    }

    /// Ask for `term` of `offer` for the owner's account. Asking again for the
    /// same term returns the same open request; a different term replaces it,
    /// and the old link stops working.
    pub async fn ask(
        &self,
        keypair: &AgentKeypair,
        offer: &str,
        term: Term,
    ) -> Result<MarketRequest, MarketError> {
        if offer.is_empty() || offer.len() > MAX_OFFER_BYTES {
            return Err(MarketError::InvalidRequest(format!(
                "An offer ID is 1 to {MAX_OFFER_BYTES} bytes."
            )));
        }
        let token = self.token(keypair).await?;
        let body = serde_json::to_vec(&Ask {
            offer,
            term: term.months(),
        })
        .map_err(|e| MarketError::InvalidRequest(e.to_string()))?;

        let response = self
            .send(keypair, Method::POST, REQUESTS_PATH, &body, &token)
            .await?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => return Err(MarketError::UnknownOffer(offer.to_string())),
            _ => return Err(refusal(&response)),
        }
        let request: MarketRequest = response
            .json()
            .await
            .map_err(|e| MarketError::InvalidResponse(e.to_string()))?;
        // The link is shown to a person to pay; nothing but an https link is
        // one the market would have sent.
        if !request.url.starts_with("https://") {
            return Err(MarketError::InvalidResponse(
                "the payment link is not an https link".into(),
            ));
        }
        Ok(request)
    }

    /// Withdraw the account's open request `request_id`; its link stops
    /// working.
    pub async fn withdraw(
        &self,
        keypair: &AgentKeypair,
        request_id: &str,
    ) -> Result<Withdrawal, MarketError> {
        // The ID becomes part of the signed path, so it may hold nothing
        // that a URL parser would read as more than one path segment.
        let plain = !request_id.is_empty()
            && request_id.len() <= MAX_REQUEST_ID_BYTES
            && request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !plain {
            return Err(MarketError::InvalidRequest(
                "A request ID holds only letters, digits, `_` and `-`.".into(),
            ));
        }
        let token = self.token(keypair).await?;
        let path = format!("{REQUESTS_PATH}/{request_id}");

        let response = self
            .send(keypair, Method::DELETE, &path, &[], &token)
            .await?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(Withdrawal::Withdrawn),
            StatusCode::NOT_FOUND => Ok(Withdrawal::NotOpen),
            _ => Err(refusal(&response)),
        }
    }

    /// A token minted for this request. The market's 401 explains nothing, so
    /// the one cause that can be seen from here, a delegation without the
    /// entitlement, is reported before the market is asked.
    async fn token(&self, keypair: &AgentKeypair) -> Result<String, MarketError> {
        let token = self.identity.mint(keypair).await?;
        if !token.entitlements.iter().any(|e| e == REQUEST_ENTITLEMENT) {
            return Err(MarketError::NotDelegated);
        }
        Ok(token.token)
    }

    /// Send one signed request, waiting out the account's link cooldown once.
    async fn send(
        &self,
        keypair: &AgentKeypair,
        method: Method,
        path: &str,
        body: &[u8],
        token: &str,
    ) -> Result<Response, MarketError> {
        let mut waited = false;
        loop {
            // Signed anew on every send: the market refuses a nonce it has
            // seen, and the wait moves the time.
            let proof = proof::header(
                keypair,
                method.as_str(),
                path,
                body,
                token,
                unix_now(),
                rand::random(),
            );
            let mut request = self
                .http
                .request(method.clone(), format!("{}{path}", self.origin))
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header(proof::HEADER, proof);
            if !body.is_empty() {
                request = request
                    .header(CONTENT_TYPE, "application/json")
                    .body(body.to_vec());
            }
            let response = request.send().await?;
            if response.status() != StatusCode::TOO_MANY_REQUESTS {
                return Ok(response);
            }
            let retry_after = retry_after(&response);
            if waited {
                return Err(MarketError::TooManyRequests { retry_after });
            }
            waited = true;
            tokio::time::sleep(Duration::from_secs(retry_after)).await;
        }
    }
}

/// The origin a market URL names, or why the token may not be sent there.
fn market_origin(market_url: &str) -> Result<String, MarketError> {
    let invalid = |why: &str| MarketError::InvalidEndpoint(format!("{market_url}: {why}"));
    let url = reqwest::Url::parse(market_url).map_err(|e| invalid(&e.to_string()))?;
    let host = url.host_str().ok_or_else(|| invalid("no host"))?;
    let loopback = host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => return Err(invalid("only https, or http on this machine")),
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("an origin only, without a path or query"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("no credentials in the address"));
    }
    Ok(url.origin().ascii_serialization())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Seconds the market asks to wait, held to the range it documents.
fn retry_after(response: &Response) -> u64 {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(MAX_RETRY_AFTER_SECS)
        .clamp(1, MAX_RETRY_AFTER_SECS)
}

/// What a refusal means, for the statuses both routes share.
fn refusal(response: &Response) -> MarketError {
    match response.status() {
        StatusCode::BAD_REQUEST => MarketError::BadRequest,
        StatusCode::UNAUTHORIZED => MarketError::Unauthorized(unauthorized_cause(response)),
        StatusCode::CONFLICT => MarketError::Conflict,
        StatusCode::SERVICE_UNAVAILABLE => MarketError::Unavailable,
        other => MarketError::Unexpected(other.as_u16()),
    }
}

/// The likeliest reason for a 401, which the market itself never gives. A
/// clock more than a minute off is certain to be refused and shows in the
/// answer's `Date`; the rest can only be named.
fn unauthorized_cause(response: &Response) -> String {
    let skew = response
        .headers()
        .get(DATE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| chrono::DateTime::parse_from_rfc2822(v).ok())
        .map(|market| chrono::Utc::now().timestamp() - market.timestamp());
    match skew {
        Some(skew) if skew.abs() > CLOCK_TOLERANCE_SECS => format!(
            "This machine's clock is {} seconds {} the market's, and the market allows \
             {CLOCK_TOLERANCE_SECS}. Correct the clock and ask again.",
            skew.abs(),
            if skew > 0 { "ahead of" } else { "behind" },
        ),
        _ => format!(
            "The usual causes: identity has not named {MARKET_URL} in this agent's token, or \
             the owner's delegation changed while the request was on its way. Ask again; if it \
             repeats, have the owner authorize the agent again."
        ),
    }
}
