use crate::discovery::IdentityEndpoints;
use crate::error::IdentityError;
use crate::pkce::Pkce;

pub const CREATOR_BUNDLE_ID: &str = "com.arkavo.ArkavoCreator";
pub const CLIENT_ID: &str = "arkavo-edge";
pub const SCOPE: &str = "openid offline_access";

pub fn authorize_url(endpoints: &IdentityEndpoints, pkce: &Pkce, redirect_uri: &str) -> String {
    // Query encoding must use %20 for space, not form-urlencoded `+`.
    let query = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", redirect_uri),
        ("scope", SCOPE),
        ("code_challenge", pkce.challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("state", pkce.state.as_str()),
    ]
    .into_iter()
    .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
    .collect::<Vec<_>>()
    .join("&");
    let base = endpoints.authorization_endpoint.trim_end_matches('?');
    if base.contains('?') {
        format!("{base}&{query}")
    } else {
        format!("{base}?{query}")
    }
}

fn percent_encode(input: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

pub fn creator_url(authorize_query_url: &str) -> String {
    let query = authorize_query_url
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("");
    format!("arkavocreator://oauth/authorize?{query}")
}

pub async fn launch_creator(creator_url: &str) -> Result<(), IdentityError> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        match Command::new("open")
            .args(["-b", CREATOR_BUNDLE_ID, creator_url])
            .status()
        {
            Ok(status) if status.success() => Ok(()),
            _ => Err(IdentityError::LoginRequired(
                "install Arkavo Creator and run 'arkavo login'".into(),
            )),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = creator_url;
        Err(IdentityError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::IdentityEndpoints;
    use crate::pkce::Pkce;

    #[cfg(not(target_os = "macos"))]
    use crate::error::IdentityError;

    #[test]
    fn creator_url_rewrites_scheme_and_keeps_query() {
        let https = "https://identity.arkavo.net/oauth/authorize?response_type=code&client_id=arkavo-edge&redirect_uri=http://127.0.0.1:52171/cb&scope=openid%20offline_access&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM&code_challenge_method=S256&state=abc";
        let u = creator_url(https);
        assert!(u.starts_with("arkavocreator://oauth/authorize?"));
        assert!(u.contains("client_id=arkavo-edge"));
        assert!(u.contains("code_challenge_method=S256"));
        assert!(!u.contains("https://"));
    }

    #[test]
    fn authorize_url_includes_pkce_and_offline_access() {
        let ep = IdentityEndpoints {
            authorization_endpoint: "https://identity.arkavo.net/oauth/authorize".into(),
            token_endpoint: "https://identity.arkavo.net/oauth/token".into(),
        };
        let pkce = Pkce {
            verifier: "v".into(),
            challenge: "c".into(),
            state: "s".into(),
        };
        let url = authorize_url(&ep, &pkce, "http://127.0.0.1:52171/cb");
        assert!(url.contains("response_type=code"));
        assert!(
            url.contains("scope=openid%20offline_access"),
            "scope must use %20, got {url}"
        );
        assert!(
            !url.contains("scope=openid+offline_access"),
            "scope must not use +, got {url}"
        );
        assert!(url.contains("code_challenge=c"));
    }

    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn launch_is_unsupported_off_macos() {
        let err = launch_creator("arkavocreator://oauth/authorize")
            .await
            .unwrap_err();
        assert!(matches!(err, IdentityError::Unsupported));
    }
}
