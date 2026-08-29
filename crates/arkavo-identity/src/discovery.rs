use crate::error::IdentityError;
use opentdf::kas_discovery::fetch_well_known;

pub const DEFAULT_PLATFORM_URL: &str = "https://platform.arkavo.net";
pub const DEFAULT_IDENTITY_HOST: &str = "identity.arkavo.net";

#[derive(Debug)]
pub struct IdentityEndpoints {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
}

pub async fn discover(
    http: &reqwest::Client,
    platform_url: &str,
    identity_host: &str,
) -> Result<IdentityEndpoints, IdentityError> {
    let cfg = fetch_well_known(platform_url, http)
        .await
        .map_err(|e| IdentityError::Transport(e.to_string()))?;
    let idp = cfg.idp.ok_or_else(|| {
        IdentityError::Transport("well-known configuration is missing idp".into())
    })?;
    let authorization_endpoint = idp.authorization_endpoint.ok_or_else(|| {
        IdentityError::Transport("well-known idp is missing authorization_endpoint".into())
    })?;
    let token_endpoint = idp.token_endpoint.ok_or_else(|| {
        IdentityError::Transport("well-known idp is missing token_endpoint".into())
    })?;
    pin_host(&authorization_endpoint, identity_host)?;
    pin_host(&token_endpoint, identity_host)?;
    Ok(IdentityEndpoints {
        authorization_endpoint,
        token_endpoint,
    })
}

pub fn host_of(url: &str) -> Result<String, IdentityError> {
    let parsed = url::Url::parse(url).map_err(|e| IdentityError::Transport(e.to_string()))?;
    parsed
        .host_str()
        .map(str::to_owned)
        .ok_or_else(|| IdentityError::Transport(format!("URL is missing a host: {url}")))
}

fn pin_host(url: &str, identity_host: &str) -> Result<(), IdentityError> {
    let host = host_of(url)?;
    if host.eq_ignore_ascii_case(identity_host) {
        Ok(())
    } else {
        Err(IdentityError::UntrustedIdentityEndpoint(host))
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // #[tokio::test] expands to Runtime::block_on
mod tests {
    use super::*;

    async fn serve_well_known(
        body: serde_json::Value,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let json = serde_json::to_vec(&body).unwrap();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let json = json.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 512];
                    loop {
                        let n = stream.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        json.len()
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(&json).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        (addr, handle)
    }

    #[tokio::test]
    async fn pins_identity_host_and_rejects_a_foreign_idp() {
        let (addr, handle) = serve_well_known(serde_json::json!({
            "idp": {
                "issuer": "https://evil.example",
                "authorization_endpoint": "https://evil.example/oauth/authorize",
                "token_endpoint": "https://evil.example/oauth/token"
            },
            "kas": { "uri": "https://platform.arkavo.net" }
        }))
        .await;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        let err = discover(&http, &format!("http://{addr}"), "identity.arkavo.net")
            .await
            .expect_err("foreign IdP must be refused");
        assert!(matches!(err, IdentityError::UntrustedIdentityEndpoint(_)));
        handle.abort();
    }

    #[tokio::test]
    async fn accepts_matching_identity_host() {
        let (addr, handle) = serve_well_known(serde_json::json!({
            "idp": {
                "issuer": "https://identity.arkavo.net",
                "authorization_endpoint": "https://identity.arkavo.net/oauth/authorize",
                "token_endpoint": "https://identity.arkavo.net/oauth/token"
            },
            "kas": { "uri": "https://platform.arkavo.net" }
        }))
        .await;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        let ep = discover(&http, &format!("http://{addr}"), "identity.arkavo.net")
            .await
            .expect("matching host is accepted");
        assert_eq!(
            ep.authorization_endpoint,
            "https://identity.arkavo.net/oauth/authorize"
        );
        assert_eq!(ep.token_endpoint, "https://identity.arkavo.net/oauth/token");
        handle.abort();
    }

    #[tokio::test]
    async fn missing_idp_is_transport() {
        let (addr, handle) = serve_well_known(serde_json::json!({
            "kas": { "uri": "https://platform.arkavo.net" }
        }))
        .await;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        let err = discover(&http, &format!("http://{addr}"), "identity.arkavo.net")
            .await
            .expect_err("missing idp must fail");
        assert!(matches!(err, IdentityError::Transport(_)));
        handle.abort();
    }

    #[tokio::test]
    async fn accepts_case_insensitive_identity_host() {
        let (addr, handle) = serve_well_known(serde_json::json!({
            "idp": {
                "issuer": "https://identity.arkavo.net",
                "authorization_endpoint": "https://identity.arkavo.net/oauth/authorize",
                "token_endpoint": "https://identity.arkavo.net/oauth/token"
            },
            "kas": { "uri": "https://platform.arkavo.net" }
        }))
        .await;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        let ep = discover(&http, &format!("http://{addr}"), "IDENTITY.ARKAVO.NET")
            .await
            .expect("host compare is case-insensitive");
        assert_eq!(
            ep.authorization_endpoint,
            "https://identity.arkavo.net/oauth/authorize"
        );
        handle.abort();
    }

    #[tokio::test]
    async fn rejects_when_only_token_endpoint_is_foreign() {
        let (addr, handle) = serve_well_known(serde_json::json!({
            "idp": {
                "issuer": "https://identity.arkavo.net",
                "authorization_endpoint": "https://identity.arkavo.net/oauth/authorize",
                "token_endpoint": "https://evil.example/oauth/token"
            },
            "kas": { "uri": "https://platform.arkavo.net" }
        }))
        .await;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        let err = discover(&http, &format!("http://{addr}"), "identity.arkavo.net")
            .await
            .expect_err("foreign token endpoint must be refused");
        assert!(matches!(
            err,
            IdentityError::UntrustedIdentityEndpoint(ref host) if host == "evil.example"
        ));
        handle.abort();
    }

    #[test]
    fn host_of_extracts_host() {
        assert_eq!(
            host_of("https://identity.arkavo.net/oauth/token").unwrap(),
            "identity.arkavo.net"
        );
        assert!(matches!(
            host_of("not a url"),
            Err(IdentityError::Transport(_))
        ));
    }

    #[test]
    fn defaults_are_arkavo_hosts() {
        assert_eq!(DEFAULT_PLATFORM_URL, "https://platform.arkavo.net");
        assert_eq!(DEFAULT_IDENTITY_HOST, "identity.arkavo.net");
    }
}
