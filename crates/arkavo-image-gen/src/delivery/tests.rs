use super::*;
use crate::testing::png;
use sha2::{Digest, Sha256};

fn entry(bytes: &[u8], width: u32, height: u32) -> AssetEntry {
    AssetEntry {
        name: "coin_000.png".into(),
        bytes: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(bytes)),
        width,
        height,
        has_alpha: true,
        tileable: false,
        seed: 1,
    }
}

fn item(count: u32) -> ImageRequest {
    ImageRequest {
        prompt: "x".into(),
        count: Some(count),
        ..ImageRequest::default()
    }
}

/// ASSET-002 edge: more than 16 images is refused before any member is
/// asked.
#[arkavo_test_macros::spec("ASSET-002")]
#[test]
fn requests_are_bounded() {
    let ok = AssetRequest {
        items: vec![item(4), item(4), item(4), item(4)],
        deadline_secs: 600,
        requester: None,
    };
    assert!(ok.check().is_ok());
    let too_many = AssetRequest {
        items: vec![item(4), item(4), item(4), item(4), item(1)],
        ..ok.clone()
    };
    assert_eq!(too_many.check().unwrap_err().code(), "invalid_params");
    let empty = AssetRequest {
        items: vec![],
        ..ok
    };
    assert!(empty.check().is_err());
}

/// ASSET-007: size, hash and dimensions are all checked.
#[arkavo_test_macros::spec("ASSET-007")]
#[test]
fn fetched_bytes_must_match_the_manifest() {
    let good = png(64, 64, true);
    let e = entry(&good, 64, 64);
    assert!(verify_fetched(&e, &good).is_ok());
    let mut tampered = good.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert!(
        verify_fetched(&e, &tampered)
            .unwrap_err()
            .to_string()
            .contains("SHA-256")
    );
    assert!(
        verify_fetched(&e, &good[..20])
            .unwrap_err()
            .to_string()
            .contains("bytes")
    );
    let wrong_size = entry(&good, 32, 64);
    assert!(
        verify_fetched(&wrong_size, &good)
            .unwrap_err()
            .to_string()
            .contains("64x64")
    );
    let not_png = b"0123456789".to_vec();
    assert!(verify_fetched(&entry(&not_png, 1, 1), &not_png).is_err());
}

#[arkavo_test_macros::spec("ASSET-007")]
#[test]
fn names_that_could_escape_are_refused() {
    assert!(safe_name("coin_000.png").is_ok());
    for bad in [
        "../coin.png",
        "a/b.png",
        ".hidden.png",
        "coin.exe",
        "",
        "co in.png",
    ] {
        assert!(safe_name(bad).is_err(), "{bad}");
    }
}

#[arkavo_test_macros::spec("ASSET-005")]
#[test]
fn deliveries_round_trip_and_find_entries() {
    let bytes = png(8, 8, false);
    let manifest: Manifest = serde_json::from_value(serde_json::json!({
        "model": {"id": "m", "repo": "r", "revision": "v", "diffusion": "d", "text_encoder": "t"},
        "license": "l", "license_url": "u", "runtime": "rt", "prompt": "p",
        "assets": [entry(&bytes, 8, 8)]
    }))
    .unwrap();
    let delivery = Delivery {
        manifest,
        references: vec![AssetRef {
            name: "coin_000.png".into(),
            ticket: "blobticket".into(),
            content_hash: "h".into(),
            media_type: PNG.into(),
        }],
    };
    let json = serde_json::to_value(&delivery).unwrap();
    assert_eq!(json["model"]["id"], "m");
    assert_eq!(json["references"][0]["ticket"], "blobticket");
    assert!(!json.to_string().contains("base64"));
    let back: Delivery = serde_json::from_value(json).unwrap();
    assert_eq!(back, delivery);
    assert_eq!(back.entry(&back.references[0]).unwrap().width, 8);
}
