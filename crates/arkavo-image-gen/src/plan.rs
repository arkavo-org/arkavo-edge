//! Which entries a host offers, with which weights, and whether a request
//! fits (IMG-002, IMG-018).

use crate::catalog::{Entry, WeightSet, entries};
use crate::error::{ImageGenError, Result};

/// The size weight sets are chosen at: 1 MP with guidance 1.
pub const BASE_PIXELS: u64 = 1024 * 1024;

/// An entry this host offers, with the weight set it loads.
#[derive(Debug, Clone, Copy)]
pub struct Offer {
    pub entry: &'static Entry,
    pub set: &'static WeightSet,
    budget: u64,
}

/// For each entry, the largest set whose weights and base working set fit
/// `budget`; entries with no fitting set are left out.
pub fn offers(budget: u64) -> Vec<Offer> {
    entries()
        .iter()
        .filter_map(|entry| {
            let base = entry.allowance(BASE_PIXELS, false)?;
            entry
                .sets
                .iter()
                .find(|set| set.bytes().saturating_add(base) <= budget)
                .map(|set| Offer { entry, set, budget })
        })
        .collect()
}

impl Offer {
    #[cfg(any(test, feature = "testing"))]
    pub(crate) const fn for_test(
        entry: &'static Entry,
        set: &'static WeightSet,
        budget: u64,
    ) -> Self {
        Self { entry, set, budget }
    }

    /// The largest pixel count this host can render, `None` when no
    /// measured size fits at this guidance.
    pub fn largest_pixels(&self, guided: bool) -> Option<u64> {
        self.entry
            .allowances
            .iter()
            .filter(|a| a.guided == guided && self.fits(a.bytes))
            .map(|a| a.pixels.min(self.entry.limits.max_pixels))
            .max()
    }

    /// Refuse a run this host cannot hold, naming what it can.
    pub fn check(&self, pixels: u64, guided: bool) -> Result<()> {
        match self.entry.allowance(pixels, guided) {
            Some(bytes) if self.fits(bytes) => Ok(()),
            _ => Err(ImageGenError::TooLargeForHost {
                entry: self.entry.id,
                pixels,
                guided: if guided { " with guidance above 1" } else { "" },
                largest: self.largest_pixels(guided).map_or_else(
                    || "nothing at this guidance".to_string(),
                    |p| format!("{p} pixels"),
                ),
            }),
        }
    }

    fn fits(&self, allowance: u64) -> bool {
        self.set.bytes().saturating_add(allowance) <= self.budget
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::SetLabel;
    use crate::host::{Host, Platform};

    const GIB: u64 = 1 << 30;

    fn budget(memory_gib: u64, chat_bytes: u64) -> u64 {
        Host {
            platform: Platform::AppleSilicon,
            chip: None,
            memory_bytes: memory_gib * GIB,
        }
        .budget(chat_bytes)
    }

    fn labels(offers: &[Offer]) -> Vec<(&str, SetLabel)> {
        offers.iter().map(|o| (o.entry.id, o.set.label)).collect()
    }

    /// The IMG-001 edge cases, with Gemma 4 12B (7.2 GB) resident.
    #[arkavo_test_macros::spec("IMG-002")]
    #[test]
    fn sets_follow_the_budget() {
        let gemma_12b = 7_200_000_000;
        assert_eq!(
            labels(&offers(budget(32, gemma_12b))),
            [
                ("qwen-image-2.1-turbo", SetLabel::L),
                ("qwen-image-2.1", SetLabel::L)
            ]
        );
        assert_eq!(
            labels(&offers(budget(64, gemma_12b))),
            [
                ("qwen-image-2.1-turbo", SetLabel::Xl),
                ("qwen-image-2.1", SetLabel::Xl)
            ]
        );
        assert_eq!(
            labels(&offers(budget(24, gemma_12b))),
            [
                ("qwen-image-2.1-turbo", SetLabel::S),
                ("qwen-image-2.1", SetLabel::S)
            ]
        );
        assert!(offers(budget(16, 3_000_000_000)).is_empty());
    }

    /// IMG-018: on a 32 GiB M6 every preset size up to 4.3 MP is accepted,
    /// with guidance for the full model; a host whose budget holds the
    /// weights but not their working set is refused, naming what it can do.
    #[arkavo_test_macros::spec("IMG-018")]
    #[test]
    fn requests_are_checked_against_the_measured_table() {
        let largest = 2400 * 1792;
        let [turbo, full] = offers(budget(32, 7_200_000_000))[..] else {
            panic!("both entries on 32 GiB");
        };
        assert!(turbo.check(largest, false).is_ok());
        assert!(full.check(largest, true).is_ok());
        assert_eq!(turbo.largest_pixels(false), Some(largest));
        assert_eq!(full.largest_pixels(true), Some(largest));
        assert_eq!(turbo.largest_pixels(true), None, "turbo runs at CFG 1");

        let tight = Offer::for_test(full.entry, full.set, full.set.bytes() + GIB / 2);
        let err = tight.check(1024 * 1024, false).unwrap_err();
        assert_eq!(err.code(), "too_large_for_host");
        assert!(
            err.to_string().contains("nothing at this guidance"),
            "{err}"
        );
        assert_eq!(tight.largest_pixels(false), None);
    }
}
