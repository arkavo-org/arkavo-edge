//! `arkavo model download` for the image catalog (CLI-020): the runtime and
//! the weight set this host would load, fetched and verified ahead of the
//! first request.

use std::fmt::Write as _;

use arkavo_image_gen::catalog::{self, ALIASES, Entry, entries};
use arkavo_image_gen::host::Platform;
use arkavo_image_gen::plan::BASE_PIXELS;
use arkavo_image_gen::{Generator, Host, Progress, Store, offers};

const GIB: f64 = (1u64 << 30) as f64;

/// Whether `name` is an image catalog id or alias.
pub fn is_image_model(name: &str) -> bool {
    catalog::resolve(name).is_some()
}

/// The chat model an agent on this device keeps resident, which the image
/// budget leaves room for (IMG-001).
fn resident_chat_bytes() -> u64 {
    crate::first_run::detect_capabilities()
        .recommended_model
        .model_choice()
        .size_bytes()
}

/// Why this host offers no image generation at all, if it does not.
fn platform_refusal(host: &Host) -> Option<&'static str> {
    match host.platform {
        Platform::AppleSilicon => None,
        Platform::Translated => Some(
            "this arkavo binary runs under Rosetta; image generation needs the native arm64 build",
        ),
        Platform::Other => Some("image generation runs on macOS with Apple Silicon only"),
    }
}

/// The memory `entry`'s smallest weight set needs with its base working set.
fn smallest_need(entry: &Entry) -> u64 {
    let weights = entry.sets.iter().map(|s| s.bytes()).min().unwrap_or(0);
    weights + entry.allowance(BASE_PIXELS, false).unwrap_or(0)
}

/// One line per catalog entry: the weight set and download size this host
/// would get, or why it gets none.
pub fn listing(host: &Host, budget: u64, store: Option<&Store>) -> String {
    let offered = offers(budget);
    let mut out = String::from("Image models (Qwen-Image, local stable-diffusion.cpp on Metal):\n");
    for entry in entries() {
        let aliases: Vec<&str> = ALIASES
            .iter()
            .filter(|(_, id)| *id == entry.id)
            .map(|(alias, _)| *alias)
            .collect();
        let named = if aliases.is_empty() {
            entry.id.to_string()
        } else {
            format!("{} ({})", entry.id, aliases.join(", "))
        };
        let detail = match (
            platform_refusal(host),
            offered.iter().find(|o| o.entry.id == entry.id),
        ) {
            (Some(reason), _) => format!("not on this host: {reason}"),
            (None, None) => format!(
                "not on this host: its memory budget is {:.1} GiB, under the {:.1} GiB the smallest set needs",
                budget as f64 / GIB,
                smallest_need(entry) as f64 / GIB
            ),
            (None, Some(offer)) => {
                let remaining = store.map_or(offer.set.bytes(), |s| s.missing_bytes(offer.set));
                let label = format!("{:?}", offer.set.label).to_uppercase();
                format!("{label} set, {:.1} GB to download", remaining as f64 / 1e9)
            }
        };
        let _ = writeln!(out, "  {named} - {} - {detail}", entry.summary);
    }
    out
}

/// The listing for the machine this runs on.
pub fn this_host_listing() -> String {
    let host = Host::detect();
    let store = Store::default_root().map(Store::new);
    listing(&host, host.budget(resident_chat_bytes()), store.as_ref())
}

/// Fetch and verify the runtime and `name`'s weights for this host.
pub fn download(name: &str) -> anyhow::Result<()> {
    let host = Host::detect();
    let resident = resident_chat_bytes();
    if let Some(reason) = platform_refusal(&host) {
        anyhow::bail!("nothing was fetched: {reason}");
    }
    let resolved =
        catalog::resolve(name).ok_or_else(|| anyhow::anyhow!("unknown image model '{name}'"))?;
    let refusal = || {
        anyhow::anyhow!(
            "nothing was fetched: this host's memory budget is {:.1} GiB ({:.0} GiB unified memory, \
             less the resident chat model and 4 GiB for the OS), under the {:.1} GiB {} needs",
            host.budget(resident) as f64 / GIB,
            host.memory_bytes as f64 / GIB,
            smallest_need(resolved.entry) as f64 / GIB,
            resolved.entry.id
        )
    };
    let generator = Generator::for_this_host(resident, None).ok_or_else(refusal)?;
    let offer = generator.offer(resolved.entry.id).ok_or_else(refusal)?;
    println!(
        "Fetching {} ({:?} set) and the stable-diffusion.cpp runtime",
        resolved.entry.id, offer.set.label
    );
    let report = |p: Progress| {
        if let Progress::Fetching { fetched, needed } = p {
            eprint!(
                "\r  {:.2} of {:.2} GB",
                fetched as f64 / 1e9,
                needed as f64 / 1e9
            );
        }
    };
    #[allow(clippy::disallowed_methods)]
    tokio::runtime::Runtime::new()?.block_on(generator.prefetch(name, &report))?;
    eprintln!();
    println!("{} is ready on this host", resolved.entry.id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    const GIB_BYTES: u64 = 1 << 30;

    fn mac(memory_gib: u64) -> Host {
        Host {
            platform: Platform::AppleSilicon,
            chip: Some("Apple M6".into()),
            memory_bytes: memory_gib * GIB_BYTES,
        }
    }

    /// CLI-020: the listing names each entry with the set and download
    /// size this host would get; IMG-001: 32 GiB with Gemma 4 12B resident
    /// gets the large sets.
    #[spec("CLI-020", "IMG-001")]
    #[test]
    fn a_32_gib_m6_lists_the_large_sets() {
        let host = mac(32);
        let gemma_12b = 7_200_000_000;
        let listing = listing(&host, host.budget(gemma_12b), None);
        assert!(
            listing.contains("qwen-image-2.1-turbo (qwen-image)"),
            "{listing}"
        );
        assert!(
            listing.contains("qwen-image-2.1 (qwen-image-quality)"),
            "{listing}"
        );
        assert_eq!(listing.matches("L set").count(), 2, "{listing}");
        assert!(listing.contains("GB to download"), "{listing}");
    }

    /// CLI-020: a host that does not qualify says why, by platform or by
    /// memory budget.
    #[spec("CLI-020")]
    #[test]
    fn hosts_that_cannot_generate_say_why() {
        let linux = Host {
            platform: Platform::Other,
            chip: None,
            memory_bytes: 64 * GIB_BYTES,
        };
        let listing = listing(&linux, linux.budget(0), None);
        assert!(
            listing.contains("macOS with Apple Silicon only"),
            "{listing}"
        );

        let small = mac(16);
        let listing = listing_for_budget(&small);
        assert!(listing.contains("under the 12.2 GiB"), "{listing}");
    }

    fn listing_for_budget(host: &Host) -> String {
        listing(host, host.budget(7_200_000_000), None)
    }

    #[test]
    fn image_names_are_recognized() {
        for name in [
            "qwen-image",
            "qwen-image-quality",
            "qwen-image-2.1-turbo",
            "qwen-image-2.1",
        ] {
            assert!(is_image_model(name), "{name}");
        }
        assert!(!is_image_model("gemma-4-12b"));
    }
}
