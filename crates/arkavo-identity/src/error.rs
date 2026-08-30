//! Identity session errors and KAS-denied message mapping.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    Interactive,
    Never,
}

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("login required; {0}")]
    LoginRequired(String),
    #[error("access denied by user")]
    AccessDenied,
    #[error("passkey login is only supported on macOS with Arkavo Creator")]
    Unsupported,
    #[error("untrusted identity endpoint: {0}")]
    UntrustedIdentityEndpoint(String),
    #[error("{0}")]
    Transport(String),
    #[error("{0}")]
    Token(String),
    #[error("{0}")]
    Store(String),
    #[error("timed out waiting for Arkavo Creator")]
    TimedOut,
}

impl IdentityError {
    pub fn kas_denied_message(&self) -> String {
        let rest = match self {
            Self::Unsupported => {
                "login required; install Arkavo Creator and run 'arkavo login'".to_string()
            }
            Self::LoginRequired(reason) => format!("login required; {reason}"),
            Self::TimedOut => "login required; timed out waiting for Arkavo Creator".to_string(),
            Self::AccessDenied => "access denied by user".to_string(),
            Self::UntrustedIdentityEndpoint(_) => "untrusted identity endpoint".to_string(),
            Self::Transport(e) | Self::Token(e) | Self::Store(e) => e.clone(),
        };
        format!("GGUFTDF_KAS_DENIED: {rest}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kas_denied_messages_match_the_spec_table() {
        assert_eq!(
            IdentityError::Unsupported.kas_denied_message(),
            "GGUFTDF_KAS_DENIED: login required; install Arkavo Creator and run 'arkavo login'"
        );
        assert_eq!(
            IdentityError::LoginRequired("run 'arkavo login'".into()).kas_denied_message(),
            "GGUFTDF_KAS_DENIED: login required; run 'arkavo login'"
        );
        assert_eq!(
            IdentityError::TimedOut.kas_denied_message(),
            "GGUFTDF_KAS_DENIED: login required; timed out waiting for Arkavo Creator"
        );
        assert_eq!(
            IdentityError::AccessDenied.kas_denied_message(),
            "GGUFTDF_KAS_DENIED: access denied by user"
        );
        assert_eq!(
            IdentityError::UntrustedIdentityEndpoint("evil.example".into()).kas_denied_message(),
            "GGUFTDF_KAS_DENIED: untrusted identity endpoint"
        );
        assert_eq!(
            IdentityError::Transport("connection reset".into()).kas_denied_message(),
            "GGUFTDF_KAS_DENIED: connection reset"
        );
    }
}
