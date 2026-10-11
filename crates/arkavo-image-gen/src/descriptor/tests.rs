use super::*;

fn row(id: &str, aliases: &[&str], set: SetLabel, ready: bool, speed: Option<f64>) -> ModelRow {
    ModelRow {
        id: id.to_string(),
        aliases: aliases.iter().map(|a| (*a).to_string()).collect(),
        weight_set: set,
        ready,
        bytes_remaining: if ready { 0 } else { 10 },
        largest_pixels: 1 << 20,
        largest_pixels_guided: None,
        steps: 8,
        seconds_per_step: speed,
    }
}

fn member(rows: Vec<ModelRow>, queue: usize) -> Descriptor {
    Descriptor {
        chip: Some("Apple M6".into()),
        unified_memory_gib: 32.0,
        runtime_build: 956,
        queue_depth: queue,
        models: rows,
    }
}

fn need(model: &str, pixels: u64, guided: bool) -> Need {
    Need {
        model: model.to_string(),
        pixels: Some(pixels),
        guided,
        images: 1,
    }
}

const TURBO: &str = "qwen-image-2.1-turbo";

/// ASSET-002: a row serves a request named by id or by the alias the member
/// resolves, when its measured size covers the request.
#[arkavo_test_macros::spec("ASSET-002")]
#[test]
fn rows_match_by_name_and_size() {
    let m = member(
        vec![row(TURBO, &["qwen-image"], SetLabel::L, true, None)],
        0,
    );
    assert!(m.row(&need(TURBO, 1 << 20, false)).is_some());
    assert!(m.row(&need("qwen-image", 512 * 512, false)).is_some());
    assert!(m.row(&need(TURBO, 4 << 20, false)).is_none());
    assert!(m.row(&need(TURBO, 1 << 20, true)).is_none());
    assert!(m.row(&need("qwen-image-2.1", 1 << 20, false)).is_none());
    let unknown_size = Need {
        pixels: None,
        ..need(TURBO, 0, false)
    };
    assert!(m.row(&unknown_size).is_some());
}

/// ASSET-002: ready first, then the larger set, then the shorter estimate
/// (unmeasured last), then the shorter queue; members that cannot serve the
/// request are left out.
#[arkavo_test_macros::spec("ASSET-002")]
#[test]
fn members_rank_by_readiness_set_time_and_queue() {
    let n = need(TURBO, 1 << 20, false);
    let ranked = rank(
        vec![
            (
                "not-ready",
                member(vec![row(TURBO, &[], SetLabel::Xl, false, Some(1.0))], 0),
            ),
            (
                "small-fast",
                member(vec![row(TURBO, &[], SetLabel::S, true, Some(1.0))], 0),
            ),
            (
                "large-slow",
                member(vec![row(TURBO, &[], SetLabel::L, true, Some(9.0))], 0),
            ),
            (
                "large-fast",
                member(vec![row(TURBO, &[], SetLabel::L, true, Some(2.0))], 0),
            ),
            (
                "large-unmeasured",
                member(vec![row(TURBO, &[], SetLabel::L, true, None)], 0),
            ),
            (
                "other-model",
                member(
                    vec![row("qwen-image-2.1", &[], SetLabel::Xl, true, None)],
                    0,
                ),
            ),
        ],
        &n,
    );
    assert_eq!(
        ranked,
        [
            "large-fast",
            "large-slow",
            "large-unmeasured",
            "small-fast",
            "not-ready"
        ]
    );
}

#[arkavo_test_macros::spec("ASSET-002")]
#[test]
fn queues_break_ties_and_count_in_estimates() {
    let n = need(TURBO, 1 << 20, false);
    let busy = member(vec![row(TURBO, &[], SetLabel::L, true, None)], 3);
    let idle = member(vec![row(TURBO, &[], SetLabel::L, true, None)], 0);
    assert_eq!(
        rank(vec![("busy", busy), ("idle", idle)], &n),
        ["idle", "busy"]
    );
    let measured = member(vec![row(TURBO, &[], SetLabel::L, true, Some(0.5))], 2);
    // 8 steps at 0.5 s for one image, plus two queued ahead of it.
    assert_eq!(measured.estimate(&n), Some(12.0));
}

#[arkavo_test_macros::spec("ASSET-002")]
#[test]
fn needs_come_from_the_request() {
    let known = Need::of(&ImageRequest {
        prompt: "x".into(),
        ..ImageRequest::default()
    });
    assert_eq!(known.model, "qwen-image");
    assert_eq!(known.pixels, Some(1 << 20));
    let future = Need::of(&ImageRequest {
        prompt: "x".into(),
        model: Some("qwen-image-3.0".into()),
        width: Some(512),
        height: Some(512),
        count: Some(2),
        ..ImageRequest::default()
    });
    assert_eq!(future.pixels, Some(512 * 512));
    assert_eq!(future.images, 2);
    let future_default_size = Need::of(&ImageRequest {
        prompt: "x".into(),
        model: Some("qwen-image-3.0".into()),
        ..ImageRequest::default()
    });
    assert_eq!(future_default_size.pixels, None);
}

#[arkavo_test_macros::spec("ASSET-001")]
#[test]
fn descriptors_round_trip_as_json() {
    let m = member(
        vec![row(TURBO, &["qwen-image"], SetLabel::L, true, Some(1.5))],
        1,
    );
    let json = serde_json::to_value(&m).unwrap();
    assert_eq!(json["models"][0]["weight_set"], "l");
    assert_eq!(serde_json::from_value::<Descriptor>(json).unwrap(), m);
}
