//! Which destinations an agent-path HTTP request may reach (NET-007, NET-014).
//!
//! The built-in blocks come from [`EgressFilter`]. The only way past them is an
//! origin the operator names in `ARKAVO_EGRESS_ALLOW` on this machine; nothing
//! an agent, a manifest or a peer supplies can widen it.

use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, OnceLock};

use url::{Host, Url};

use crate::url::{EgressError, EgressFilter};

/// Operator allowlist: comma-separated origins such as `http://localhost:3000`.
pub const EGRESS_ALLOW_ENV: &str = "ARKAVO_EGRESS_ALLOW";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum AllowedHost {
    Ip(IpAddr),
    Domain(String),
}

/// Built-in blocked ranges plus the origins the operator exempted from them.
pub struct EgressPolicy {
    filter: EgressFilter,
    allowed: BTreeSet<(AllowedHost, u16)>,
}

impl EgressPolicy {
    /// The built-in blocks with nothing exempted.
    #[must_use]
    pub fn strict() -> Self {
        Self {
            filter: EgressFilter::new(),
            allowed: BTreeSet::new(),
        }
    }

    /// The built-in blocks, exempting each origin in `spec`.
    ///
    /// Entries are bare origins. A path, query or credentials would read as a
    /// narrower grant than the host-and-port one actually given, so they are
    /// refused rather than ignored.
    pub fn with_allowlist(spec: &str) -> Result<Self, EgressError> {
        let mut policy = Self::strict();
        for entry in spec.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            policy.allowed.insert(parse_origin(entry)?);
        }
        Ok(policy)
    }

    /// The policy for this process: built-in blocks plus `ARKAVO_EGRESS_ALLOW`,
    /// read once.
    ///
    /// There is no uninitialised state to fall back from. Unset means nothing
    /// is exempted; a malformed value is an error every caller sees, because
    /// quietly dropping the entry would look like the filter was broken rather
    /// than the entry.
    pub fn process() -> Result<Arc<Self>, EgressError> {
        static PROCESS: OnceLock<Result<Arc<EgressPolicy>, EgressError>> = OnceLock::new();
        PROCESS
            .get_or_init(|| match std::env::var(EGRESS_ALLOW_ENV) {
                Ok(spec) => Self::with_allowlist(&spec).map(Arc::new),
                Err(std::env::VarError::NotPresent) => Ok(Arc::new(Self::strict())),
                Err(std::env::VarError::NotUnicode(_)) => Err(EgressError::InvalidAllowlistEntry(
                    format!("{EGRESS_ALLOW_ENV} (not UTF-8)"),
                )),
            })
            .clone()
    }

    /// What can be decided from the URL alone.
    ///
    /// An IP-literal host is decided here in full: the connector dials it
    /// without asking any resolver. A domain's addresses are judged when it
    /// resolves; the URL decides only the port on a domain the operator
    /// exempted, since the resolver never learns the port.
    pub fn check_url(&self, url: &Url) -> Result<(), EgressError> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(EgressError::InvalidUrl);
        }
        let port = url.port_or_known_default().ok_or(EgressError::InvalidUrl)?;
        match url.host() {
            Some(Host::Ipv4(v4)) => self.check_ip(IpAddr::V4(v4), port),
            Some(Host::Ipv6(v6)) => self.check_ip(IpAddr::V6(v6), port),
            Some(Host::Domain(domain)) => self.check_domain(domain, port),
            None => Err(EgressError::InvalidUrl),
        }
    }

    /// Judge what DNS returned for `domain`.
    ///
    /// Every address has to pass. A name answering with both public and
    /// private addresses is either misconfigured or rebinding, and in neither
    /// case can the request be shown to stay public.
    pub fn vet_resolved(
        &self,
        domain: &str,
        addrs: Vec<SocketAddr>,
    ) -> Result<Vec<SocketAddr>, EgressError> {
        if self.domain_exempt(&normalize_domain(domain)) {
            return Ok(addrs);
        }
        if let Some(blocked) = addrs
            .iter()
            .map(SocketAddr::ip)
            .find(|ip| self.filter.validate_resolved_ip(*ip).is_err())
        {
            return Err(EgressError::BlockedIp(blocked));
        }
        Ok(addrs)
    }

    /// Judge `url` the way a request to it would be judged, resolution
    /// included, for a caller whose connection some other process makes.
    ///
    /// The answer covers this lookup only; whatever connects resolves again
    /// and can be told something different.
    pub async fn vet_destination(&self, url: &Url) -> Result<(), EgressError> {
        self.check_url(url)?;
        let (Some(Host::Domain(domain)), Some(port)) = (url.host(), url.port_or_known_default())
        else {
            return Ok(());
        };
        let addrs = tokio::net::lookup_host((domain, port))
            .await
            .map_err(|_| EgressError::Unresolved(domain.to_string()))?
            .collect();
        self.vet_resolved(domain, addrs).map(|_| ())
    }

    fn check_ip(&self, ip: IpAddr, port: u16) -> Result<(), EgressError> {
        if self.allowed.contains(&(AllowedHost::Ip(ip), port)) {
            return Ok(());
        }
        self.filter.validate_resolved_ip(ip)
    }

    fn check_domain(&self, domain: &str, port: u16) -> Result<(), EgressError> {
        let domain = normalize_domain(domain);
        if self.domain_exempt(&domain)
            && !self
                .allowed
                .contains(&(AllowedHost::Domain(domain.clone()), port))
        {
            return Err(EgressError::BlockedDomain(format!("{domain}:{port}")));
        }
        Ok(())
    }

    fn domain_exempt(&self, domain: &str) -> bool {
        self.allowed
            .iter()
            .any(|(host, _)| matches!(host, AllowedHost::Domain(d) if d == domain))
    }
}

fn parse_origin(entry: &str) -> Result<(AllowedHost, u16), EgressError> {
    let invalid = || EgressError::InvalidAllowlistEntry(entry.to_string());
    let url = Url::parse(entry).map_err(|_| invalid())?;
    let bare = matches!(url.scheme(), "http" | "https")
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    if !bare {
        return Err(invalid());
    }
    let port = url.port_or_known_default().ok_or_else(invalid)?;
    let host = match url.host() {
        Some(Host::Ipv4(v4)) => AllowedHost::Ip(IpAddr::V4(v4)),
        Some(Host::Ipv6(v6)) => AllowedHost::Ip(IpAddr::V6(v6)),
        Some(Host::Domain(domain)) => AllowedHost::Domain(normalize_domain(domain)),
        None => return Err(invalid()),
    };
    Ok((host, port))
}

/// `localhost.` and `LOCALHOST` reach the same host as `localhost`; an
/// exemption keyed on one spelling must hold, and hold only, for all of them.
fn normalize_domain(domain: &str) -> String {
    domain.trim_end_matches('.').to_ascii_lowercase()
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[spec("NET-007")]
    #[test]
    fn strict_policy_blocks_loopback_and_metadata_literals() {
        let policy = EgressPolicy::strict();
        for target in [
            "http://127.0.0.1/",
            "http://[::1]:8080/",
            "http://169.254.169.254/latest/meta-data/",
            "http://2130706433/",
            "http://0x7f.1/",
            "http://[::ffff:169.254.169.254]/",
        ] {
            assert!(
                matches!(
                    policy.check_url(&url(target)),
                    Err(EgressError::BlockedIp(_))
                ),
                "{target} must be blocked"
            );
        }
    }

    #[spec("NET-007")]
    #[test]
    fn non_http_schemes_are_refused() {
        let policy = EgressPolicy::strict();
        for target in [
            "file:///etc/passwd",
            "ftp://example.com/",
            "gopher://example.com/",
        ] {
            assert_eq!(policy.check_url(&url(target)), Err(EgressError::InvalidUrl));
        }
    }

    #[spec("NET-014")]
    #[test]
    fn a_name_resolving_to_metadata_is_refused() {
        let policy = EgressPolicy::strict();
        let metadata: SocketAddr = "169.254.169.254:0".parse().unwrap();
        assert_eq!(
            policy.vet_resolved("metadata.attacker.test", vec![metadata]),
            Err(EgressError::BlockedIp(metadata.ip()))
        );
    }

    #[spec("NET-014")]
    #[test]
    fn a_mixed_public_and_private_answer_is_refused() {
        let policy = EgressPolicy::strict();
        let public: SocketAddr = "93.184.215.14:0".parse().unwrap();
        let private: SocketAddr = "10.0.0.7:0".parse().unwrap();
        assert_eq!(
            policy.vet_resolved("rebind.attacker.test", vec![public, private]),
            Err(EgressError::BlockedIp(private.ip()))
        );
        assert_eq!(
            policy.vet_resolved("example.com", vec![public]),
            Ok(vec![public])
        );
    }

    #[spec("NET-018")]
    #[test]
    fn an_allowlisted_origin_is_exempt_on_its_port_only() {
        let policy =
            EgressPolicy::with_allowlist("http://localhost:3000, http://127.0.0.1:8080").unwrap();
        assert!(policy.check_url(&url("http://localhost:3000/app")).is_ok());
        assert!(policy.check_url(&url("http://LOCALHOST.:3000/")).is_ok());
        assert!(
            policy
                .check_url(&url("http://127.0.0.1:8080/health"))
                .is_ok()
        );
        assert_eq!(
            policy.check_url(&url("http://localhost:22/")),
            Err(EgressError::BlockedDomain("localhost:22".into()))
        );
        assert!(matches!(
            policy.check_url(&url("http://127.0.0.1:8081/")),
            Err(EgressError::BlockedIp(_))
        ));
        let loopback: SocketAddr = "127.0.0.1:0".parse().unwrap();
        assert!(policy.vet_resolved("localhost", vec![loopback]).is_ok());
        assert!(policy.vet_resolved("other.test", vec![loopback]).is_err());
    }

    #[spec("NET-018")]
    #[test]
    fn a_malformed_allowlist_entry_is_named_in_the_error() {
        for entry in [
            "localhost:3000",
            "http://localhost:3000/admin",
            "http://user:pw@localhost:3000",
            "ftp://localhost:21",
        ] {
            let spec = format!("http://localhost:4000,{entry}");
            assert_eq!(
                EgressPolicy::with_allowlist(&spec).err(),
                Some(EgressError::InvalidAllowlistEntry(entry.to_string())),
                "{entry} must be refused"
            );
        }
    }

    #[spec("NET-018")]
    #[test]
    fn an_empty_allowlist_is_strict() {
        let policy = EgressPolicy::with_allowlist("").unwrap();
        assert!(policy.check_url(&url("http://127.0.0.1:3000/")).is_err());
    }

    #[spec("NET-014")]
    #[tokio::test]
    async fn vet_destination_decides_literals_without_a_lookup() {
        let policy = EgressPolicy::strict();
        assert!(matches!(
            policy
                .vet_destination(&url("http://169.254.169.254/latest/meta-data/"))
                .await,
            Err(EgressError::BlockedIp(_))
        ));
        assert_eq!(
            policy.vet_destination(&url("file:///etc/passwd")).await,
            Err(EgressError::InvalidUrl)
        );
        assert_eq!(
            policy.vet_destination(&url("http://93.184.215.14/")).await,
            Ok(())
        );
    }

    #[spec("NET-014")]
    #[tokio::test]
    async fn vet_destination_judges_what_a_name_resolves_to() {
        // `localhost` resolves from the hosts file, so no network is involved.
        let strict = EgressPolicy::strict();
        assert!(matches!(
            strict.vet_destination(&url("http://localhost:9/")).await,
            Err(EgressError::BlockedIp(_))
        ));

        let allowed = EgressPolicy::with_allowlist("http://localhost:3000").unwrap();
        assert_eq!(
            allowed
                .vet_destination(&url("http://localhost:3000/"))
                .await,
            Ok(())
        );
        assert_eq!(
            allowed.vet_destination(&url("http://localhost:22/")).await,
            Err(EgressError::BlockedDomain("localhost:22".into()))
        );
    }
}
