//! Which routes are protected, and the policy each one requires.

use arkavo_tap::RoutePolicy;
use axum::http::Method;

/// A path pattern: literal segments and `{name}` placeholders for one segment each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathPattern(Vec<Segment>);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Any,
}

impl PathPattern {
    /// Parses `/demo/v1/products/{id}`.
    pub fn parse(pattern: &str) -> Result<Self, String> {
        let rest = pattern
            .strip_prefix('/')
            .ok_or_else(|| format!("pattern {pattern:?} must start with /"))?;
        let segments = rest
            .split('/')
            .map(|s| {
                if s.starts_with('{') && s.ends_with('}') && s.len() > 2 {
                    Ok(Segment::Any)
                } else if s.contains(['{', '}']) || s.is_empty() {
                    Err(format!("pattern {pattern:?} has an invalid segment {s:?}"))
                } else {
                    Ok(Segment::Literal(s.to_owned()))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PathPattern(segments))
    }

    /// Whether `path` (without its query) matches. A placeholder matches one non-empty
    /// segment.
    pub fn matches(&self, path: &str) -> bool {
        let Some(rest) = path.strip_prefix('/') else {
            return false;
        };
        let parts: Vec<&str> = rest.split('/').collect();
        parts.len() == self.0.len()
            && self.0.iter().zip(&parts).all(|(seg, part)| match seg {
                Segment::Literal(l) => l == part,
                Segment::Any => !part.is_empty(),
            })
    }
}

/// A protected route: requests matching it are admitted only after verification.
#[derive(Debug, Clone)]
pub struct ProtectedRoute {
    pub method: Method,
    pub path: PathPattern,
    pub policy: RoutePolicy,
}

impl ProtectedRoute {
    pub fn new(method: Method, pattern: &str, policy: RoutePolicy) -> Result<Self, String> {
        Ok(ProtectedRoute {
            method,
            path: PathPattern::parse(pattern)?,
            policy,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_match_whole_segments() {
        let p = PathPattern::parse("/demo/v1/products/{id}").unwrap();
        assert!(p.matches("/demo/v1/products/demo-001"));
        assert!(!p.matches("/demo/v1/products/"));
        assert!(!p.matches("/demo/v1/products/a/b"));
        assert!(!p.matches("/demo/v1/product/demo-001"));
        assert!(!p.matches("demo/v1/products/x"));
        let exact = PathPattern::parse("/demo/v1/orders").unwrap();
        assert!(exact.matches("/demo/v1/orders"));
        assert!(!exact.matches("/demo/v1/orders/"));
    }

    #[test]
    fn invalid_patterns_are_refused() {
        for bad in ["demo", "/a//b", "/a/{}", "/a/b{c}", "/"] {
            assert!(PathPattern::parse(bad).is_err(), "{bad}");
        }
    }
}
