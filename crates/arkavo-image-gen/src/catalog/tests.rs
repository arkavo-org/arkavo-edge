// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use super::*;
use std::collections::HashSet;

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn all_files() -> Vec<&'static PinnedFile> {
    entries()
        .iter()
        .flat_map(|e| e.sets.iter())
        .flat_map(WeightSet::files)
        .collect()
}

/// IMG-017: ids and aliases are unique and every alias names an entry.
#[arkavo_test_macros::spec("IMG-017", "IMG-019")]
#[test]
fn ids_and_aliases_are_unique_and_resolve() {
    let mut seen = HashSet::new();
    for name in names() {
        assert!(seen.insert(name), "{name} is used twice");
    }
    for (alias, id) in ALIASES {
        let resolved = resolve(alias).unwrap_or_else(|| panic!("{alias} names no entry"));
        assert_eq!(resolved.entry.id, *id);
        assert_eq!(resolved.alias, Some(*alias));
    }
    let default = resolve(DEFAULT_MODEL).expect("default model resolves");
    assert_eq!(default.entry.id, "qwen-image-2.1-turbo");
    assert!(resolve("qwen-image-9").is_none());
}

/// IMG-017: no entry needs a newer runtime than the pinned one.
#[arkavo_test_macros::spec("IMG-017")]
#[test]
fn entries_run_on_the_pinned_runtime() {
    for entry in entries() {
        assert!(
            entry.min_runtime_build <= RUNTIME.build,
            "{} needs runtime build {}, pinned is {}",
            entry.id,
            entry.min_runtime_build,
            RUNTIME.build
        );
    }
}

/// IMG-017: every pin is a commit, a SHA-256 and a size.
#[arkavo_test_macros::spec("IMG-016", "IMG-017")]
#[test]
fn pins_are_commits_hashes_and_sizes() {
    for file in all_files() {
        assert!(is_hex(file.revision, 40), "{}: revision", file.path);
        assert!(is_hex(file.sha256, 64), "{}: sha256", file.path);
        assert!(file.size > 0, "{}: size", file.path);
        assert!(file.repo.contains('/'), "{}: repo", file.path);
        assert!(file.url().contains(file.revision));
    }
    for entry in entries() {
        assert!(
            is_hex(entry.model_revision, 40),
            "{}: model revision",
            entry.id
        );
    }
    assert!(is_hex(RUNTIME.sha256, 64));
    assert!(RUNTIME.url.contains(RUNTIME.tag));
    assert!(RUNTIME.members.contains(&RUNTIME.cli));
}

/// IMG-002 relies on sets being ordered largest first.
#[arkavo_test_macros::spec("IMG-017")]
#[test]
fn sets_are_ordered_largest_first() {
    for entry in entries() {
        assert!(!entry.sets.is_empty(), "{} has no weight set", entry.id);
        for pair in entry.sets.windows(2) {
            assert!(
                pair[0].label < pair[1].label,
                "{}: labels out of order",
                entry.id
            );
            assert!(
                pair[0].bytes() > pair[1].bytes(),
                "{}: sizes out of order",
                entry.id
            );
        }
    }
}

/// IMG-007: limits are whole steps and the default size is inside them.
#[arkavo_test_macros::spec("IMG-007", "IMG-017")]
#[test]
fn limits_are_consistent() {
    for entry in entries() {
        let l = &entry.limits;
        for side in [l.min_side, l.max_side, l.default_width, l.default_height] {
            assert!(
                side.is_multiple_of(l.step),
                "{}: {side} is not a multiple of {}",
                entry.id,
                l.step
            );
            assert!(
                (l.min_side..=l.max_side).contains(&side),
                "{}: {side}",
                entry.id
            );
        }
        let default_pixels = u64::from(l.default_width) * u64::from(l.default_height);
        assert!(
            default_pixels <= l.max_pixels,
            "{}: default too large",
            entry.id
        );
        if let Some(max) = entry.profile.max_guidance {
            assert!(max > 1.0, "{}: guidance ceiling", entry.id);
        }
    }
}

/// IMG-002 chooses sets with the 1 MP unguided allowance, so every entry
/// needs one.
#[arkavo_test_macros::spec("IMG-018")]
#[test]
fn every_entry_has_a_base_allowance() {
    for entry in entries() {
        assert!(
            entry.allowance(1024 * 1024, false).is_some(),
            "{}",
            entry.id
        );
    }
}

/// IMG-018: the smallest measured point covering a size applies, and an
/// unmeasured size or guidance has no allowance.
#[arkavo_test_macros::spec("IMG-018")]
#[test]
fn allowance_lookup_takes_the_covering_point() {
    let entry = resolve("qwen-image").unwrap().entry;
    assert_eq!(entry.allowance(512 * 512, false), Some(3 << 30));
    assert_eq!(entry.allowance(1024 * 1024, false), Some(3 << 30));
    assert_eq!(entry.allowance(2048 * 2048, false), None);
    assert_eq!(entry.allowance(512 * 512, true), None);
    assert_eq!(entry.largest_measured(false), Some(1024 * 1024));
    assert_eq!(entry.largest_measured(true), None);
}

/// Shared files are stored once, under a name that keeps the extension.
#[arkavo_test_macros::spec("IMG-004")]
#[test]
fn store_names_dedupe_shared_files() {
    let names = pinned_store_names();
    let distinct: HashSet<_> = all_files().iter().map(|f| f.sha256).collect();
    assert_eq!(names.len(), distinct.len());
    for name in &names {
        assert!(
            name.ends_with(".gguf") || name.ends_with(".safetensors"),
            "{name}"
        );
    }
}

/// The weight set sizes the spec quotes (IMG-002).
#[arkavo_test_macros::spec("IMG-002", "IMG-016")]
#[test]
fn weight_set_sizes_match_the_spec() {
    let turbo = resolve("qwen-image-2.1-turbo").unwrap().entry;
    let sizes: Vec<u64> = turbo.sets.iter().map(WeightSet::bytes).collect();
    assert_eq!(
        sizes,
        [
            31_293_830_392,
            16_976_583_928,
            13_294_849_272,
            9_904_989_432
        ]
    );
    let full = resolve("qwen-image-2.1").unwrap().entry;
    let sizes: Vec<u64> = full.sets.iter().map(WeightSet::bytes).collect();
    assert_eq!(
        sizes,
        [
            31_293_835_200,
            17_072_184_888,
            13_390_450_232,
            9_900_789_304
        ]
    );
}

/// IMG-017: every pinned weight file still resolves at its commit with the
/// size and hash the catalog names. Needs the network.
#[arkavo_test_macros::spec("IMG-017")]
#[tokio::test]
#[ignore = "requires network access to huggingface.co"]
async fn pins_match_the_hub() {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("HTTP client");
    let mut checked = HashSet::new();
    for file in all_files() {
        if !checked.insert(file.sha256) {
            continue;
        }
        let response = client.head(file.url()).send().await.expect("HEAD");
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .trim_matches('"')
                .to_string()
        };
        assert_eq!(header("x-repo-commit"), file.revision, "{}", file.path);
        assert_eq!(
            header("x-linked-size"),
            file.size.to_string(),
            "{}",
            file.path
        );
        assert_eq!(header("x-linked-etag"), file.sha256, "{}", file.path);
    }
}
