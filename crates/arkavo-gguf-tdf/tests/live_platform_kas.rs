//! Live wrap/unlock against platform.arkavo.net + identity.arkavo.net.
//!
//! Off by default (`#[ignore]`). Run:
//!   cargo test -p arkavo-gguf-tdf --features kas --test live_platform_kas -- --ignored --nocapture

#![cfg(feature = "kas")]

use arkavo_gguf_tdf::GgufTdfArchive;
use opentdf::kas::KasClient;
use opentdf::kas_discovery::{OpentdfConfiguration, fetch_well_known};
use std::path::Path;

#[tokio::test]
#[ignore]
async fn rewrap_through_identity_arkavo_net_is_denied() {
    let archive_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/gguf-tdf-test/tiny.gguf.tdf");
    assert!(
        archive_path.exists(),
        "wrap a model first: {}",
        archive_path.display()
    );

    let archive = GgufTdfArchive::open(&archive_path).expect("archive must open structurally");
    let kas_in_manifest = archive
        .manifest()
        .encryption_information
        .key_access
        .first()
        .map(|k| k.url.as_str())
        .unwrap_or("<missing>");
    println!("archive {}", archive_path.display());
    println!("manifest kas.url {kas_in_manifest}");
    println!("virtual {} bytes", archive.virtual_size());

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();

    let well_known = fetch_well_known("https://platform.arkavo.net", &http)
        .await
        .expect("platform well-known must fetch");
    let token_url = well_known
        .idp
        .as_ref()
        .and_then(|idp| idp.token_endpoint.as_deref())
        .unwrap_or("https://identity.arkavo.net/oauth/token");
    println!("idp token_endpoint {token_url}");

    let token_response = http
        .post(token_url)
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", "arkavo-edge"),
        ])
        .send()
        .await
        .expect("identity.arkavo.net must be reachable");
    let token_status = token_response.status();
    let token_body = token_response.text().await.unwrap_or_default();
    println!("identity token HTTP {token_status}: {token_body}");
    assert!(
        !token_status.is_success(),
        "anonymous client_credentials must not mint a token"
    );

    let cfg = OpentdfConfiguration::for_kas_connect("https://platform.arkavo.net");
    let kas = KasClient::new(&cfg, "").expect("KasClient constructs with an empty bearer");
    let err = kas
        .rewrap_standard_tdf(archive.manifest())
        .await
        .expect_err("rewrap without a platform identity must fail");
    println!("rewrap error: {err}");
    let msg = err.to_string();
    assert!(
        msg.contains("401")
            || msg.to_lowercase().contains("auth")
            || msg.to_lowercase().contains("denied")
            || msg.to_lowercase().contains("forbidden"),
        "expected an authn/authz failure, got: {msg}"
    );
}

/// Positive spike: an authorization-code access CWT must unwrap the archive.
///
/// Requires:
///   - `arkavo-edge` registered on identity.arkavo.net (see design spec)
///   - `ARKAVO_IDENTITY_CWT` = Creator's session CWT (operator-supplied;
///     this test never reads the Keychain)
///
/// Run:
///   ARKAVO_IDENTITY_CWT=... cargo test -p arkavo-gguf-tdf --features kas \
///     --test live_platform_kas authorization_code_access_token_unwraps \
///     -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn authorization_code_access_token_unwraps_tiny_gguf_tdf() {
    let cwt = match std::env::var("ARKAVO_IDENTITY_CWT") {
        Ok(v) if !v.is_empty() => v,
        _ => panic!("set ARKAVO_IDENTITY_CWT to Creator's session CWT"),
    };
    let archive_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/gguf-tdf-test/tiny.gguf.tdf");
    let archive = GgufTdfArchive::open(&archive_path).expect("archive must open structurally");

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let well_known = fetch_well_known("https://platform.arkavo.net", &http)
        .await
        .expect("platform well-known must fetch");
    let authorize = well_known
        .idp
        .as_ref()
        .and_then(|idp| idp.authorization_endpoint.as_deref())
        .unwrap_or("https://identity.arkavo.net/oauth/authorize");
    let token_url = well_known
        .idp
        .as_ref()
        .and_then(|idp| idp.token_endpoint.as_deref())
        .unwrap_or("https://identity.arkavo.net/oauth/token");

    // RFC 7636 appendix B verifier so a human can recompute the challenge.
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    let redirect = "http://127.0.0.1:52171/cb";
    let state = "spike-state";

    let authorize_url = format!(
        "{authorize}?response_type=code&client_id=arkavo-edge&redirect_uri={redirect}\
         &scope=openid%20offline_access&code_challenge={challenge}\
         &code_challenge_method=S256&state={state}"
    );
    let authz = http
        .get(&authorize_url)
        .header("X-Auth-Token", &cwt)
        .send()
        .await
        .expect("authorize reachable");
    let authz_status = authz.status();
    let location = authz
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let authz_body = authz.text().await.unwrap_or_default();
    println!("authorize HTTP {authz_status} location={location} body={authz_body}");
    assert!(
        authz_status.as_u16() == 307 || authz_status.is_redirection(),
        "expected 307 to loopback, got {authz_status} {authz_body} \
         (register OIDC_CLIENT_EDGE_* if this is invalid_client)"
    );
    assert!(
        location.starts_with(redirect),
        "Location must start with the request redirect_uri: {location}"
    );
    let code = location
        .split(['?', '&'])
        .find_map(|p| p.strip_prefix("code="))
        .expect("Location must carry code=");

    let token_response = http
        .post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", "arkavo-edge"),
            ("redirect_uri", redirect),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .expect("token endpoint reachable");
    let token_status = token_response.status();
    let token_json: serde_json::Value = token_response.json().await.expect("token JSON");
    println!(
        "token HTTP {token_status} keys={:?}",
        token_json.as_object().map(|o| o.keys().collect::<Vec<_>>())
    );
    assert!(
        token_status.is_success(),
        "token exchange failed: {token_json}"
    );
    let access = token_json["access_token"]
        .as_str()
        .expect("access_token")
        .to_string();
    assert!(
        token_json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .is_some(),
        "offline_access must mint a refresh_token: {token_json}"
    );

    let cfg = OpentdfConfiguration::for_kas_connect("https://platform.arkavo.net");
    let kas = KasClient::new(&cfg, access).expect("KasClient");
    let key = kas
        .rewrap_standard_tdf(archive.manifest())
        .await
        .expect("KAS must unwrap with the OIDC access CWT");
    println!("payload key {} bytes", key.len());
    assert_eq!(key.len(), 32, "payload key must be AES-256");
}
