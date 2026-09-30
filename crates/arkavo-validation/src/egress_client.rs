//! The HTTP client for requests made on an agent's behalf (NET-007, NET-014).
//!
//! reqwest reaches a destination three ways, and each needs the policy applied
//! where reqwest consults it:
//! - a domain goes through the resolver, so `PolicyResolver` judges the
//!   addresses the connection will actually use and no second lookup can
//!   answer differently;
//! - an IP-literal host is dialled without any resolver, so the URL is checked
//!   before the request exists;
//! - a redirect names a new URL, so every hop is checked the same way.
//!
//! The builder deliberately exposes no resolver overrides, redirect policy or
//! proxy settings: each would be a way around one of the three.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::Policy;
use reqwest::{Client, Method, RequestBuilder};
use url::Url;

use crate::egress_policy::EgressPolicy;
use crate::url::EgressError;

/// Redirect hops followed before giving up; reqwest's own default.
const MAX_REDIRECTS: usize = 10;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

struct PolicyResolver {
    policy: Arc<EgressPolicy>,
}

impl Resolve for PolicyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let policy = Arc::clone(&self.policy);
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let found: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let vetted: Addrs = Box::new(policy.vet_resolved(&host, found)?.into_iter());
            Ok(vetted)
        })
    }
}

fn redirect_policy(policy: Arc<EgressPolicy>) -> Policy {
    Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        match policy.check_url(attempt.url()) {
            Ok(()) => attempt.follow(),
            Err(blocked) => attempt.error(blocked),
        }
    })
}

/// An HTTP client whose every destination passes the egress policy.
#[derive(Clone)]
pub struct EgressClient {
    inner: Client,
    policy: Arc<EgressPolicy>,
}

impl EgressClient {
    #[must_use]
    pub fn builder() -> EgressClientBuilder {
        EgressClientBuilder::default()
    }

    /// Start a request, refusing it before it exists if the URL alone decides.
    pub fn request(&self, method: Method, url: &str) -> Result<RequestBuilder, EgressError> {
        let url = Url::parse(url).map_err(|_| EgressError::InvalidUrl)?;
        self.policy.check_url(&url)?;
        Ok(self.inner.request(method, url))
    }

    pub fn get(&self, url: &str) -> Result<RequestBuilder, EgressError> {
        self.request(Method::GET, url)
    }

    pub fn post(&self, url: &str) -> Result<RequestBuilder, EgressError> {
        self.request(Method::POST, url)
    }

    pub fn put(&self, url: &str) -> Result<RequestBuilder, EgressError> {
        self.request(Method::PUT, url)
    }
}

/// Configures an [`EgressClient`]; only settings that cannot move a request
/// somewhere the policy did not judge are offered.
#[derive(Default)]
pub struct EgressClientBuilder {
    timeout: Option<Duration>,
    user_agent: Option<String>,
    policy: Option<Arc<EgressPolicy>>,
}

impl EgressClientBuilder {
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    #[must_use]
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /// Judge destinations by `policy` instead of [`EgressPolicy::process`].
    #[must_use]
    pub fn policy(mut self, policy: Arc<EgressPolicy>) -> Self {
        self.policy = Some(policy);
        self
    }

    pub fn build(self) -> Result<EgressClient, EgressError> {
        let policy = match self.policy {
            Some(policy) => policy,
            None => EgressPolicy::process()?,
        };
        let mut builder = Client::builder()
            .use_rustls_tls()
            .connect_timeout(CONNECT_TIMEOUT)
            .dns_resolver(Arc::new(PolicyResolver {
                policy: Arc::clone(&policy),
            }))
            .redirect(redirect_policy(Arc::clone(&policy)))
            // A proxy resolves the destination itself, out of this resolver's
            // sight: through one, the check would judge the proxy's address
            // and never the target's.
            .no_proxy();
        if let Some(timeout) = self.timeout {
            builder = builder.timeout(timeout);
        }
        if let Some(user_agent) = self.user_agent {
            builder = builder.user_agent(user_agent);
        }
        let inner = builder
            .build()
            .map_err(|e| EgressError::Client(e.to_string()))?;
        Ok(EgressClient { inner, policy })
    }
}
