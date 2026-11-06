//! The request as the signer addressed it: method, external origin, path, query and header
//! fields, and the RFC 9421 component values derived from them.
//!
//! The authority and scheme come from the merchant's configured external origin, never from
//! `Host`, `Forwarded` or `X-Forwarded-*`: a client controls those, and a signature verified
//! against the client's own claim of where it was sent proves nothing about this merchant.

use crate::error::Rejection;
use url::Url;

/// A merchant's external origin: scheme and authority, with the default port omitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalOrigin {
    scheme: String,
    authority: String,
}

impl ExternalOrigin {
    /// Parses `https://host[:port]` (or `http://` for loopback development). A path other
    /// than `/`, a query, a fragment or user information is refused.
    pub fn parse(origin: &str) -> Result<Self, String> {
        let url = Url::parse(origin).map_err(|e| format!("origin {origin:?}: {e}"))?;
        Self::from_url(&url, true)
    }

    fn from_url(url: &Url, origin_only: bool) -> Result<Self, String> {
        let scheme = url.scheme();
        if scheme != "https" && scheme != "http" {
            return Err(format!("origin scheme must be http or https, not {scheme}"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("origin must not carry user information".into());
        }
        if origin_only && (url.path() != "/" || url.query().is_some() || url.fragment().is_some()) {
            return Err("origin must be scheme and authority only".into());
        }
        let host = url.host_str().ok_or("origin has no host")?;
        let authority = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        };
        Ok(ExternalOrigin {
            scheme: scheme.to_owned(),
            authority,
        })
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    pub fn authority(&self) -> &str {
        &self.authority
    }

    /// `scheme://authority`, the merchant audience a replay claim is scoped to.
    pub fn as_origin(&self) -> String {
        format!("{}://{}", self.scheme, self.authority)
    }

    /// Whether the host is a loopback address, where plain HTTP may be allowed for
    /// development.
    pub fn is_loopback(&self) -> bool {
        let host = self
            .authority
            .rsplit_once(':')
            .map_or(self.authority.as_str(), |(h, port)| {
                if port.bytes().all(|b| b.is_ascii_digit()) {
                    h
                } else {
                    &self.authority
                }
            });
        matches!(host, "127.0.0.1" | "localhost" | "[::1]")
    }
}

/// One request, as signed or as received.
#[derive(Debug, Clone)]
pub struct RequestContext {
    origin: ExternalOrigin,
    method: String,
    path: String,
    query: Option<String>,
    headers: Vec<(String, String)>,
}

impl RequestContext {
    /// A request to `origin` for `path_and_query` (origin form, as in the request line).
    /// Header names are case-insensitive and kept lower-case; values must be visible ASCII.
    pub fn new(
        origin: ExternalOrigin,
        method: &str,
        path_and_query: &str,
        headers: Vec<(String, String)>,
    ) -> Result<Self, Rejection> {
        if method.is_empty() || !method.bytes().all(is_tchar) {
            return Err(Rejection::malformed("method is not a token"));
        }
        if !path_and_query.starts_with('/')
            || path_and_query.contains('#')
            || !path_and_query.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(Rejection::malformed("request target is not in origin form"));
        }
        let (path, query) = match path_and_query.split_once('?') {
            Some((p, q)) => (p.to_owned(), Some(q.to_owned())),
            None => (path_and_query.to_owned(), None),
        };
        let mut lowered = Vec::with_capacity(headers.len());
        for (name, value) in headers {
            if name.is_empty() || !name.bytes().all(is_tchar) {
                return Err(Rejection::malformed("header name is not a token"));
            }
            if !value
                .bytes()
                .all(|b| b == b'\t' || (b' '..=b'~').contains(&b))
            {
                return Err(Rejection::malformed(format!(
                    "header {name} has a value outside visible ASCII"
                )));
            }
            lowered.push((name.to_ascii_lowercase(), value));
        }
        Ok(RequestContext {
            origin,
            method: method.to_owned(),
            path,
            query,
            headers: lowered,
        })
    }

    /// The request a client will send to `url`: its origin, path and query.
    pub fn for_url(
        method: &str,
        url: &Url,
        headers: Vec<(String, String)>,
    ) -> Result<Self, Rejection> {
        let origin = ExternalOrigin::from_url(url, false).map_err(Rejection::malformed)?;
        if url.fragment().is_some() {
            return Err(Rejection::malformed("a request URL has no fragment"));
        }
        let target = match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_owned(),
        };
        Self::new(origin, method, &target, headers)
    }

    pub fn origin(&self) -> &ExternalOrigin {
        &self.origin
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    /// `scheme://authority/path?query`: the `@target-uri` value.
    pub fn target_uri(&self) -> String {
        let mut uri = format!("{}{}", self.origin.as_origin(), self.path);
        if let Some(q) = &self.query {
            uri.push('?');
            uri.push_str(q);
        }
        uri
    }

    /// Every value of a header field, in order.
    pub fn header_values(&self, name: &str) -> Vec<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// A header field's values, each trimmed and joined with `", "`, as RFC 9421 section
    /// 2.1 combines them. `None` when the field is absent.
    pub fn header(&self, name: &str) -> Option<String> {
        let values = self.header_values(name);
        if values.is_empty() {
            return None;
        }
        Some(
            values
                .iter()
                .map(|v| v.trim_matches([' ', '\t']))
                .collect::<Vec<_>>()
                .join(", "),
        )
    }

    /// Total bytes of header names and values, for the header-size limit.
    pub fn header_bytes(&self) -> usize {
        self.headers.iter().map(|(n, v)| n.len() + v.len()).sum()
    }

    /// The value of a covered component. Only plain derived components and header fields
    /// are supported; component parameters are refused before this is called.
    pub fn component(&self, id: &str) -> Result<String, Rejection> {
        match id {
            "@method" => Ok(self.method.clone()),
            "@authority" => Ok(self.origin.authority.clone()),
            "@scheme" => Ok(self.origin.scheme.clone()),
            "@target-uri" => Ok(self.target_uri()),
            "@path" => Ok(if self.path.is_empty() {
                "/".to_owned()
            } else {
                self.path.clone()
            }),
            "@query" => Ok(format!("?{}", self.query.as_deref().unwrap_or(""))),
            derived if derived.starts_with('@') => Err(Rejection::malformed(format!(
                "derived component {derived} is not supported"
            ))),
            field => self
                .header(field)
                .ok_or_else(|| Rejection::malformed(format!("covered field {field} is absent"))),
        }
    }
}

fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin() -> ExternalOrigin {
        ExternalOrigin::parse("https://Market.Arkavo.com").unwrap()
    }

    #[test]
    fn origins_normalize_and_refuse_extras() {
        let o = origin();
        assert_eq!(o.authority(), "market.arkavo.com");
        assert_eq!(o.as_origin(), "https://market.arkavo.com");
        assert!(!o.is_loopback());
        let local = ExternalOrigin::parse("http://127.0.0.1:3001").unwrap();
        assert_eq!(local.authority(), "127.0.0.1:3001");
        assert!(local.is_loopback());
        assert!(
            ExternalOrigin::parse("http://[::1]:80")
                .unwrap()
                .is_loopback()
        );
        assert_eq!(
            ExternalOrigin::parse("https://example.com:443")
                .unwrap()
                .authority(),
            "example.com"
        );
        for bad in [
            "ftp://example.com",
            "https://example.com/shop",
            "https://example.com/?q=1",
            "https://user:pw@example.com",
            "not a url",
        ] {
            assert!(ExternalOrigin::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn derives_rfc_9421_components() {
        let ctx = RequestContext::new(
            origin(),
            "POST",
            "/foo?param=Value&Pet=dog",
            vec![
                ("Content-Type".into(), "application/json".into()),
                ("X-Multi".into(), " a ".into()),
                ("x-multi".into(), "b\t".into()),
            ],
        )
        .unwrap();
        assert_eq!(ctx.component("@method").unwrap(), "POST");
        assert_eq!(ctx.component("@authority").unwrap(), "market.arkavo.com");
        assert_eq!(ctx.component("@scheme").unwrap(), "https");
        assert_eq!(ctx.component("@path").unwrap(), "/foo");
        assert_eq!(ctx.component("@query").unwrap(), "?param=Value&Pet=dog");
        assert_eq!(
            ctx.component("@target-uri").unwrap(),
            "https://market.arkavo.com/foo?param=Value&Pet=dog"
        );
        assert_eq!(ctx.component("content-type").unwrap(), "application/json");
        assert_eq!(ctx.component("x-multi").unwrap(), "a, b");
        assert!(ctx.component("date").is_err());
        assert!(ctx.component("@request-target").is_err());
        assert!(ctx.header_bytes() > 0);
        assert_eq!(ctx.method(), "POST");
    }

    #[test]
    fn empty_query_is_a_question_mark() {
        let ctx = RequestContext::new(origin(), "GET", "/", vec![]).unwrap();
        assert_eq!(ctx.component("@query").unwrap(), "?");
        assert_eq!(ctx.target_uri(), "https://market.arkavo.com/");
    }

    #[test]
    fn refuses_what_cannot_be_signed_unambiguously() {
        let o = origin();
        assert!(RequestContext::new(o.clone(), "GE T", "/", vec![]).is_err());
        assert!(RequestContext::new(o.clone(), "GET", "http://x/", vec![]).is_err());
        assert!(RequestContext::new(o.clone(), "GET", "/a#b", vec![]).is_err());
        assert!(RequestContext::new(o.clone(), "GET", "/a b", vec![]).is_err());
        assert!(
            RequestContext::new(o.clone(), "GET", "/", vec![("bad name".into(), "v".into())])
                .is_err()
        );
        assert!(
            RequestContext::new(o, "GET", "/", vec![("x".into(), "caf\u{e9}".into())]).is_err()
        );
    }

    #[test]
    fn for_url_takes_the_urls_origin() {
        let url = Url::parse("http://127.0.0.1:3001/demo/v1/products/demo-001?x=1").unwrap();
        let ctx = RequestContext::for_url("GET", &url, vec![]).unwrap();
        assert_eq!(ctx.origin().authority(), "127.0.0.1:3001");
        assert_eq!(
            ctx.component("@path").unwrap(),
            "/demo/v1/products/demo-001"
        );
        let frag = Url::parse("https://example.com/a#b").unwrap();
        assert!(RequestContext::for_url("GET", &frag, vec![]).is_err());
    }
}
