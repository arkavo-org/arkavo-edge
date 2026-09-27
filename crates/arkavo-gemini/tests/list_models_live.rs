//! Live integration test for `RestClient::list_models`.
//!
//! Skipped unless `GEMINI_API_KEY` is set. Verifies that the model catalog
//! returned by the v1beta endpoint includes `gemini-3.5-flash` and that at
//! least one model advertises `generateContent` — the logical method that
//! also fronts the SSE `streamGenerateContent` path that production routes
//! through.
#![allow(clippy::disallowed_methods)]

use arkavo_gemini::RestClient;
use arkavo_test_macros::spec;

fn api_key_or_skip(test: &str) -> Option<String> {
    match std::env::var("GEMINI_API_KEY") {
        Ok(k) if !k.is_empty() => Some(k),
        _ => {
            eprintln!("skipping {test}: GEMINI_API_KEY not set");
            None
        }
    }
}

#[spec("GEM-001")]
#[tokio::test]
async fn lists_gemini_3_5_flash_and_streaming_methods() {
    let Some(api_key) = api_key_or_skip("lists_gemini_3_5_flash_and_streaming_methods") else {
        return;
    };

    let client = RestClient::new(api_key, "unused");
    let response = client.list_models(None).await.expect("list_models failed");

    assert!(!response.models.is_empty(), "expected non-empty model list");

    let has_3_5_flash = response
        .models
        .iter()
        .any(|m| m.name.contains("gemini-3.5-flash"));
    assert!(
        has_3_5_flash,
        "model catalog missing gemini-3.5-flash; got {} models",
        response.models.len()
    );

    // `streamGenerateContent` is not listed in `supportedGenerationMethods`;
    // it's an alternate HTTP path on top of `generateContent`. So assert the
    // logical method is present.
    let gencontent_count = response
        .models
        .iter()
        .filter(|m| {
            m.supported_generation_methods
                .as_ref()
                .map(|methods| methods.iter().any(|m| m == "generateContent"))
                .unwrap_or(false)
        })
        .count();
    assert!(gencontent_count > 0, "no models advertise generateContent");
}
