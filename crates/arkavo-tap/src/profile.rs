//! Signature profiles.
//!
//! A profile says which components a signature must cover, which parameters it carries, and
//! how long it may live. A profile is chosen by the route, never by the request, and a failed
//! strict verification never falls back to a weaker one.
//!
//! These are Arkavo configuration names, not Visa standards. `visa-sample-fixture` is not
//! offered: no fixtures have been captured from the public sample, and a profile without its
//! fixtures would be a guess about another implementation's behaviour.

use std::fmt;

/// The operation a route performs. Writes need more coverage than reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    /// Read a product or quote.
    Read,
    /// Create or update a checkout.
    Checkout,
    /// Commit an order or payment. Coverage alone never authorizes it.
    Commit,
}

/// The declared interaction purpose, carried in the `tag` parameter. Neither purpose is a
/// spending grant; each is only a policy input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    Browse,
    Pay,
}

impl Purpose {
    pub fn tag(self) -> &'static str {
        match self {
            Purpose::Browse => "agent-browser-auth",
            Purpose::Pay => "agent-payer-auth",
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "agent-browser-auth" => Some(Purpose::Browse),
            "agent-payer-auth" => Some(Purpose::Pay),
            _ => None,
        }
    }
}

const READ: &[&str] = &["@authority", "@path", "@method", "@target-uri"];
const WRITE: &[&str] = &[
    "@authority",
    "@path",
    "@method",
    "@target-uri",
    "content-type",
    "content-digest",
    "idempotency-key",
];
const VISA_RECOGNITION: &[&str] = &["@authority", "@path"];

/// The upper bound every profile places on `expires - created`, in seconds.
pub const MAX_LIFETIME_SECONDS: i64 = 480;

/// A named signature profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Profile {
    /// Arkavo's bilateral profile: RFC 9421 with coverage of the actual operation.
    ArkavoRfc9421V1,
    /// The agent recognition signature of the pinned Visa TAP revision (see
    /// `docs/tap/upstream.md`). Reads need only its baseline coverage; writes must cover the
    /// operation the way the Arkavo profile does, since the baseline binds neither the method,
    /// the query nor the content.
    VisaTapPinned,
}

impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Profile::ArkavoRfc9421V1 => "arkavo-rfc9421-v1",
            Profile::VisaTapPinned => "visa-tap-pinned",
        }
    }

    /// The profile with this configuration name. `visa-sample-fixture` is refused by name.
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "arkavo-rfc9421-v1" => Ok(Profile::ArkavoRfc9421V1),
            "visa-tap-pinned" => Ok(Profile::VisaTapPinned),
            "visa-sample-fixture" => Err(
                "visa-sample-fixture is not available: no fixtures have been captured from the public sample".into(),
            ),
            other => Err(format!("unknown signature profile {other:?}")),
        }
    }

    /// The `alg` value, spelled as the profile's upstream spells it.
    pub fn algorithm(self) -> &'static str {
        match self {
            Profile::ArkavoRfc9421V1 => "ed25519",
            Profile::VisaTapPinned => "Ed25519",
        }
    }

    /// Components a signature must cover for `operation`. It may cover more.
    pub fn required_components(self, operation: Operation) -> &'static [&'static str] {
        match (self, operation) {
            (Profile::VisaTapPinned, Operation::Read) => VISA_RECOGNITION,
            (_, Operation::Read) => READ,
            (_, Operation::Checkout | Operation::Commit) => WRITE,
        }
    }

    /// The longest accepted `expires - created`.
    pub fn max_lifetime(self) -> i64 {
        MAX_LIFETIME_SECONDS
    }

    /// Accepted clock skew for `created` in the future. The strict profiles allow none.
    pub fn future_tolerance(self) -> i64 {
        0
    }

    /// Whether `nonce` has the form the profile requires. Arkavo nonces carry at least 128
    /// bits: 22 or more base64url characters. The Visa revision fixes no length, so only a
    /// bounded token of visible ASCII is required there.
    pub fn nonce_is_valid(self, nonce: &str) -> bool {
        match self {
            Profile::ArkavoRfc9421V1 => {
                (22..=128).contains(&nonce.len())
                    && nonce
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            }
            Profile::VisaTapPinned => {
                (1..=256).contains(&nonce.len()) && nonce.bytes().all(|b| b.is_ascii_graphic())
            }
        }
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_the_sample_profile_is_refused() {
        for p in [Profile::ArkavoRfc9421V1, Profile::VisaTapPinned] {
            assert_eq!(Profile::parse(p.name()), Ok(p));
            assert_eq!(p.to_string(), p.name());
        }
        assert!(
            Profile::parse("visa-sample-fixture")
                .unwrap_err()
                .contains("no fixtures")
        );
        assert!(Profile::parse("weak").is_err());
    }

    #[test]
    fn writes_cover_the_content_and_idempotency_key_in_both_profiles() {
        for p in [Profile::ArkavoRfc9421V1, Profile::VisaTapPinned] {
            for op in [Operation::Checkout, Operation::Commit] {
                let c = p.required_components(op);
                for needed in [
                    "@method",
                    "@target-uri",
                    "content-digest",
                    "idempotency-key",
                ] {
                    assert!(c.contains(&needed), "{p} {op:?} {needed}");
                }
            }
        }
        assert_eq!(
            Profile::VisaTapPinned.required_components(Operation::Read),
            &["@authority", "@path"]
        );
        assert!(
            Profile::ArkavoRfc9421V1
                .required_components(Operation::Read)
                .contains(&"@target-uri")
        );
    }

    #[test]
    fn algorithms_keep_their_upstream_spelling() {
        assert_eq!(Profile::ArkavoRfc9421V1.algorithm(), "ed25519");
        assert_eq!(Profile::VisaTapPinned.algorithm(), "Ed25519");
        assert_eq!(Profile::VisaTapPinned.max_lifetime(), 480);
        assert_eq!(Profile::ArkavoRfc9421V1.future_tolerance(), 0);
    }

    #[test]
    fn purposes_map_to_tags() {
        for p in [Purpose::Browse, Purpose::Pay] {
            assert_eq!(Purpose::from_tag(p.tag()), Some(p));
        }
        assert_eq!(Purpose::from_tag("agent-checkout"), None);
    }

    #[test]
    fn nonce_forms() {
        let arkavo = Profile::ArkavoRfc9421V1;
        assert!(arkavo.nonce_is_valid("AAAAAAAAAAAAAAAAAAAAAA"));
        assert!(!arkavo.nonce_is_valid("short"));
        assert!(!arkavo.nonce_is_valid("AAAAAAAAAAAAAAAAAAAAA="));
        let visa = Profile::VisaTapPinned;
        assert!(visa.nonce_is_valid("e8N7S2MFd/qrd6T2R3tdfAuuANngKI7LFtKYI/vowzk="));
        assert!(!visa.nonce_is_valid(""));
        assert!(!visa.nonce_is_valid("has space"));
    }
}
