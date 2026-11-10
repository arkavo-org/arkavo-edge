//! `MarketClient`: asking, withdrawing, and what each refusal becomes.

use arkavo_agent_auth::{AgentAuthConfig, AgentAuthError};
use arkavo_market::{MarketClient, MarketError, Term, Withdrawal};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{OFFER, market_requests, open_request, refusal, world, world_with};

#[tokio::test]
async fn an_ask_returns_the_open_request_and_its_payment_link() {
    let w = world(vec![open_request(3)]).await;

    let request = w.client.ask(&w.keypair, OFFER, Term::Three).await.unwrap();

    assert_eq!(request.request, "rq_AAAAAAAAAAAAAAAAAAAAAA");
    assert_eq!(request.url, "https://buy.stripe.com/test_abc");
    assert_eq!(request.term, 3);
    assert_eq!(request.status, "open");

    let sent = market_requests(&w).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method.as_str(), "POST");
    assert_eq!(sent[0].url.path(), "/api/requests");
    assert_eq!(sent[0].body, br#"{"offer":"arkavo-online-play","term":3}"#);
    assert_eq!(
        sent[0].headers.get("content-type").unwrap(),
        "application/json"
    );
}

#[tokio::test]
async fn every_ask_presents_a_token_minted_for_it() {
    let w = world(vec![open_request(12)]).await;

    w.client.ask(&w.keypair, OFFER, Term::Twelve).await.unwrap();
    w.client.ask(&w.keypair, OFFER, Term::Twelve).await.unwrap();

    let tokens: Vec<String> = market_requests(&w)
        .await
        .iter()
        .map(|r| {
            r.headers
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(tokens, ["Bearer token-0", "Bearer token-1"]);
}

#[tokio::test]
async fn without_the_entitlement_the_market_is_never_called() {
    let w = world_with(
        &["https://arkavo.ai/attr/tdf/value/decrypt"],
        vec![open_request(3)],
    )
    .await;

    let err = w
        .client
        .ask(&w.keypair, OFFER, Term::Three)
        .await
        .unwrap_err();

    assert!(matches!(err, MarketError::NotDelegated), "{err}");
    assert!(err.to_string().contains("--market-requests"), "{err}");
    assert!(market_requests(&w).await.is_empty());
}

#[tokio::test]
async fn an_agent_nobody_authorized_is_told_so_by_identity() {
    let w = world(vec![open_request(3)]).await;
    w.identity.reset().await;
    Mock::given(method("GET"))
        .and(path("/agents/challenge"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Delegation not found"))
        .mount(&w.identity)
        .await;

    let err = w
        .client
        .ask(&w.keypair, OFFER, Term::Three)
        .await
        .unwrap_err();

    assert!(
        matches!(err, MarketError::Identity(AgentAuthError::NotAuthorized)),
        "{err}"
    );
    assert!(market_requests(&w).await.is_empty());
}

#[tokio::test]
async fn a_rate_limit_is_waited_out_and_asked_again_with_a_new_proof() {
    let w = world(vec![
        refusal(429, "too_many_requests").insert_header("retry-after", "1"),
        open_request(3),
    ])
    .await;

    let started = std::time::Instant::now();
    let request = w.client.ask(&w.keypair, OFFER, Term::Three).await.unwrap();

    assert_eq!(request.status, "open");
    assert!(started.elapsed() >= std::time::Duration::from_secs(1));
    let proofs: Vec<String> = market_requests(&w)
        .await
        .iter()
        .map(|r| {
            r.headers
                .get("arkavo-signature")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        })
        .collect();
    // The stand-in refuses a reused nonce, so reaching the second answer
    // already shows the retry was signed afresh.
    assert_eq!(proofs.len(), 2);
    assert_ne!(proofs[0], proofs[1]);
}

#[tokio::test]
async fn a_rate_limit_that_holds_is_reported_with_the_wait() {
    let w = world(vec![
        refusal(429, "too_many_requests").insert_header("retry-after", "1"),
    ])
    .await;

    let err = w
        .client
        .ask(&w.keypair, OFFER, Term::Three)
        .await
        .unwrap_err();

    assert!(
        matches!(err, MarketError::TooManyRequests { retry_after: 1 }),
        "{err}"
    );
    assert_eq!(market_requests(&w).await.len(), 2);
}

#[tokio::test]
async fn each_refusal_gets_its_own_message() {
    for (status, word, expected) in [
        (400, "bad_request", "offer ID and the term"),
        (401, "unauthorized", "credentials"),
        (404, "not_found", "arkavo-online-play"),
        (409, "conflict", "ask again"),
        (503, "unavailable", "unavailable"),
        (500, "error", "500"),
    ] {
        let w = world(vec![refusal(status, word)]).await;

        let err = w
            .client
            .ask(&w.keypair, OFFER, Term::Three)
            .await
            .unwrap_err();

        assert!(err.to_string().contains(expected), "{status}: {err}");
        assert_eq!(
            market_requests(&w).await.len(),
            1,
            "{status} is not retried"
        );
    }
}

#[tokio::test]
async fn a_refused_credential_names_a_wrong_clock_when_that_is_the_cause() {
    let w = world(vec![
        refusal(401, "unauthorized").insert_header("date", "Tue, 10 Nov 2020 08:00:00 GMT"),
    ])
    .await;

    let err = w
        .client
        .ask(&w.keypair, OFFER, Term::Three)
        .await
        .unwrap_err();

    assert!(matches!(err, MarketError::Unauthorized(_)), "{err}");
    assert!(err.to_string().contains("clock"), "{err}");
}

#[tokio::test]
async fn a_redirect_is_not_followed_with_the_token() {
    let elsewhere = MockServer::start().await;
    let w = world(vec![
        ResponseTemplate::new(307).insert_header("location", elsewhere.uri().as_str()),
    ])
    .await;

    let err = w
        .client
        .ask(&w.keypair, OFFER, Term::Three)
        .await
        .unwrap_err();

    assert!(matches!(err, MarketError::Unexpected(307)), "{err}");
    assert!(elsewhere.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn an_answer_whose_link_is_not_https_is_refused() {
    let w = world(vec![ResponseTemplate::new(200).set_body_json(json!({
        "request": "rq_A", "url": "http://buy.example/x", "term": 3, "status": "open",
    }))])
    .await;

    let err = w
        .client
        .ask(&w.keypair, OFFER, Term::Three)
        .await
        .unwrap_err();

    assert!(matches!(err, MarketError::InvalidResponse(_)), "{err}");
}

#[tokio::test]
async fn an_offer_the_body_limit_could_not_carry_is_refused_before_any_call() {
    let w = world(vec![open_request(3)]).await;

    for offer in [String::new(), "o".repeat(200)] {
        let err = w
            .client
            .ask(&w.keypair, &offer, Term::Three)
            .await
            .unwrap_err();
        assert!(matches!(err, MarketError::InvalidRequest(_)), "{err}");
    }
    assert!(w.identity.received_requests().await.unwrap().is_empty());
    assert!(market_requests(&w).await.is_empty());
}

#[tokio::test]
async fn a_withdrawal_signs_the_empty_body_for_the_requests_own_path() {
    let w = world(vec![ResponseTemplate::new(204)]).await;

    let outcome = w
        .client
        .withdraw(&w.keypair, "rq_AAAAAAAAAAAAAAAAAAAA-_")
        .await
        .unwrap();

    assert_eq!(outcome, Withdrawal::Withdrawn);
    let sent = market_requests(&w).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method.as_str(), "DELETE");
    assert_eq!(
        sent[0].url.path(),
        "/api/requests/rq_AAAAAAAAAAAAAAAAAAAA-_"
    );
    assert_eq!(sent[0].body, b"");
}

#[tokio::test]
async fn withdrawing_a_request_that_is_not_open_is_not_an_error() {
    let w = world(vec![refusal(404, "not_found")]).await;

    let outcome = w.client.withdraw(&w.keypair, "rq_gone").await.unwrap();

    assert_eq!(outcome, Withdrawal::NotOpen);
}

#[tokio::test]
async fn a_request_id_that_would_change_the_path_is_refused_before_any_call() {
    let w = world(vec![ResponseTemplate::new(204)]).await;

    for id in ["", "../requests", "rq_a?x=1", "rq a", "rq_a/b"] {
        let err = w.client.withdraw(&w.keypair, id).await.unwrap_err();
        assert!(
            matches!(err, MarketError::InvalidRequest(_)),
            "{id:?}: {err}"
        );
    }
    assert!(w.identity.received_requests().await.unwrap().is_empty());
    assert!(market_requests(&w).await.is_empty());
}

#[test]
fn the_token_is_sent_only_to_https_or_this_machine() {
    let identity = || AgentAuthConfig::new("https://identity.arkavo.net");
    for refused in [
        "http://market.arkavo.com",
        "http://192.168.1.10:8443",
        "https://market.arkavo.com/api",
        "https://market.arkavo.com/?x=1",
        "ftp://market.arkavo.com",
        "not a url",
    ] {
        let err = MarketClient::with_endpoints(refused, identity()).err();
        assert!(
            matches!(err, Some(MarketError::InvalidEndpoint(_))),
            "{refused}"
        );
    }
    for accepted in [
        "https://market.arkavo.com",
        "http://127.0.0.1:9000",
        "http://localhost:9000",
        "http://[::1]:9000",
    ] {
        assert!(
            MarketClient::with_endpoints(accepted, identity()).is_ok(),
            "{accepted}"
        );
    }
}

#[test]
fn a_term_is_three_or_twelve_months() {
    assert_eq!(Term::try_from(3).unwrap(), Term::Three);
    assert_eq!(Term::try_from(12).unwrap(), Term::Twelve);
    assert_eq!(Term::Twelve.months(), 12);
    for months in [0, 1, 6, 13] {
        assert!(
            matches!(Term::try_from(months), Err(MarketError::InvalidRequest(_))),
            "{months}"
        );
    }
}
