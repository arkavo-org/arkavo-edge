use super::*;

fn req(prompt: &str) -> ImageRequest {
    ImageRequest {
        prompt: prompt.to_string(),
        ..ImageRequest::default()
    }
}

fn field(err: &ImageGenError) -> &'static str {
    match err {
        ImageGenError::InvalidParams { field, .. } => field,
        other => panic!("expected InvalidParams, got {other:?}"),
    }
}

/// IMG-019: no model means the default alias, recorded as such.
#[arkavo_test_macros::spec("IMG-007", "IMG-019")]
#[test]
fn defaults_follow_the_default_entry() {
    let job = req("a grass tile").validate().unwrap();
    assert_eq!(job.entry.id, "qwen-image-2.1-turbo");
    assert_eq!(job.alias, Some("qwen-image"));
    assert_eq!((job.width, job.height, job.count), (1024, 1024, 1));
    assert_eq!(job.name, "asset");
    assert!(!job.transparent && !job.tileable && !job.guided());
    assert!((job.guidance - 1.0).abs() < f32::EPSILON);
}

#[test]
fn the_full_model_defaults_to_2k() {
    let job = ImageRequest {
        model: Some("qwen-image-quality".into()),
        ..req("castle")
    }
    .validate()
    .unwrap();
    assert_eq!(job.entry.id, "qwen-image-2.1");
    assert_eq!((job.width, job.height), (2048, 2048));
}

/// IMG-019: an unknown name lists what this host knows.
#[arkavo_test_macros::spec("IMG-019")]
#[test]
fn unknown_models_list_the_catalog() {
    let err = ImageRequest {
        model: Some("qwen-image-9".into()),
        ..req("x")
    }
    .validate()
    .unwrap_err();
    assert_eq!(err.code(), "invalid_params");
    let message = err.to_string();
    for name in [
        "qwen-image-2.1-turbo",
        "qwen-image-2.1",
        "qwen-image",
        "qwen-image-quality",
    ] {
        assert!(message.contains(name), "{message}");
    }
}

#[arkavo_test_macros::spec("IMG-007")]
#[test]
fn prompts_are_trimmed_and_bounded() {
    assert_eq!(req("  sword  ").validate().unwrap().prompt, "sword");
    assert_eq!(field(&req("   ").validate().unwrap_err()), "prompt");
    assert_eq!(
        field(&req(&"x".repeat(2001)).validate().unwrap_err()),
        "prompt"
    );
    assert!(req(&"x".repeat(2000)).validate().is_ok());
}

/// IMG-007: sizes are whole steps within the entry's limits; 2752x1536 is
/// an official preset and valid for both entries.
#[arkavo_test_macros::spec("IMG-007")]
#[test]
fn sizes_follow_the_entry_limits() {
    let sized = |w, h| ImageRequest {
        width: Some(w),
        height: Some(h),
        ..req("x")
    };
    assert!(sized(2752, 1536).validate().is_ok());
    assert!(sized(2400, 1792).validate().is_ok());
    assert!(sized(256, 256).validate().is_ok());
    assert_eq!(field(&sized(1000, 1024).validate().unwrap_err()), "width");
    assert_eq!(field(&sized(1024, 2784).validate().unwrap_err()), "height");
    assert_eq!(field(&sized(224, 256).validate().unwrap_err()), "width");
    // 2752x2752 has legal sides but exceeds the pixel limit.
    assert_eq!(field(&sized(2752, 2752).validate().unwrap_err()), "width");
    let one_side = ImageRequest {
        width: Some(512),
        ..req("x")
    }
    .validate()
    .unwrap();
    assert_eq!((one_side.width, one_side.height), (512, 1024));
}

#[arkavo_test_macros::spec("IMG-007")]
#[test]
fn count_and_name_are_checked() {
    let with = |count, name: &str| ImageRequest {
        count: Some(count),
        name: Some(name.to_string()),
        ..req("x")
    };
    assert!(with(4, "coin_gold-2").validate().is_ok());
    assert_eq!(field(&with(0, "a").validate().unwrap_err()), "count");
    assert_eq!(field(&with(5, "a").validate().unwrap_err()), "count");
    assert_eq!(field(&with(1, "../coin").validate().unwrap_err()), "name");
    assert_eq!(field(&with(1, "").validate().unwrap_err()), "name");
    assert_eq!(
        field(&with(1, &"a".repeat(65)).validate().unwrap_err()),
        "name"
    );
}

#[arkavo_test_macros::spec("IMG-007")]
#[test]
fn seeds_are_kept_or_drawn() {
    let job = ImageRequest {
        seed: Some(42),
        ..req("x")
    }
    .validate()
    .unwrap();
    assert_eq!(job.seed, 42);
    let drawn: std::collections::HashSet<u32> =
        (0..8).map(|_| req("x").validate().unwrap().seed).collect();
    assert!(drawn.len() > 1, "seeds are not random: {drawn:?}");
}

/// IMG-007 edge: Turbo refuses a negative prompt and any guidance but 1.
#[arkavo_test_macros::spec("IMG-007", "IMG-008")]
#[test]
fn turbo_refuses_guidance() {
    let neg = ImageRequest {
        negative_prompt: Some("blurry".into()),
        ..req("x")
    };
    assert_eq!(field(&neg.validate().unwrap_err()), "negative_prompt");
    let guided = ImageRequest {
        guidance: Some(4.0),
        ..req("x")
    };
    assert_eq!(field(&guided.validate().unwrap_err()), "guidance");
    let neutral = ImageRequest {
        guidance: Some(1.0),
        ..req("x")
    };
    assert!(neutral.validate().is_ok());
}

/// IMG-007: guidance above 1 and a negative prompt come together.
#[arkavo_test_macros::spec("IMG-007")]
#[test]
fn full_model_guidance_needs_a_negative_prompt() {
    let full = |guidance, negative: Option<&str>| ImageRequest {
        model: Some("qwen-image-2.1".into()),
        guidance,
        negative_prompt: negative.map(str::to_string),
        ..req("x")
    };
    let job = full(Some(4.0), Some(" blurry ")).validate().unwrap();
    assert!(job.guided());
    assert_eq!(job.negative_prompt.as_deref(), Some("blurry"));
    assert!(!full(None, None).validate().unwrap().guided());
    assert_eq!(
        field(&full(Some(4.0), None).validate().unwrap_err()),
        "negative_prompt"
    );
    assert_eq!(
        field(&full(None, Some("blurry")).validate().unwrap_err()),
        "guidance"
    );
    assert_eq!(
        field(&full(Some(10.5), Some("b")).validate().unwrap_err()),
        "guidance"
    );
    assert_eq!(
        field(&full(Some(0.5), Some("b")).validate().unwrap_err()),
        "guidance"
    );
}

/// IMG-009: transparent output uses Qwen's framing.
#[arkavo_test_macros::spec("IMG-009")]
#[test]
fn transparent_prompts_are_framed() {
    let job = ImageRequest {
        transparent: Some(true),
        ..req("A coin sprite.")
    }
    .validate()
    .unwrap();
    assert_eq!(
        job.effective_prompt(),
        "This is an RGBA image with transparency. A coin sprite. The image has alpha channel and the background is transparent."
    );
    assert_eq!(
        req("A coin sprite.").validate().unwrap().effective_prompt(),
        "A coin sprite."
    );
}

/// IMG-016 edge: editing is refused by name, not as an unknown field.
#[arkavo_test_macros::spec("IMG-016")]
#[test]
fn reference_images_are_unsupported() {
    let request: ImageRequest =
        serde_json::from_value(serde_json::json!({"prompt": "x", "reference_images": ["t"]}))
            .unwrap();
    assert_eq!(
        request.validate().unwrap_err().code(),
        "unsupported_feature"
    );
}

#[arkavo_test_macros::spec("IMG-007")]
#[test]
fn unknown_fields_are_rejected_when_parsing() {
    let parsed: std::result::Result<ImageRequest, _> =
        serde_json::from_value(serde_json::json!({"prompt": "x", "stepz": 4}));
    assert!(parsed.is_err());
}
