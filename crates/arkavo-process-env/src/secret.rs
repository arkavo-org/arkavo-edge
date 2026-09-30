//! Recognising a credential by its variable name.
//!
//! Every credential the agent reads follows the `<SCOPE>_<KIND>` convention
//! providers and CI systems use (`OPENAI_API_KEY`, `GITHUB_TOKEN`,
//! `JWT_SECRET`, `ARKAVO_MASTER_KEY`), so the last underscore-separated word
//! names the kind. A word that only ever means a secret (`SECRET`,
//! `PASSWORD`) marks the name wherever it appears.
//!
//! This is a deny-list, and it is fail-open for a credential stored under an
//! unconventional name; see `ChildEnv::toolchain` for why the toolchain
//! profile accepts that.

/// Final words that name a credential.
const KIND_WORDS: &[&str] = &[
    "KEY",
    "APIKEY",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "PASSPHRASE",
    "CREDENTIALS",
    "CREDS",
    "PEM",
    "PAT",
    "AUTH",
];

/// Words that mark a credential wherever they appear (`AWS_SECRET_ACCESS_KEY`,
/// `DB_PASSWORD_FILE`).
const ANYWHERE_WORDS: &[&str] = &["SECRET", "PASSWORD", "PASSWD"];

/// Whether an environment variable name, by convention, holds a
/// credential.
pub fn is_secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let last = upper.rsplit('_').next().unwrap_or_default();
    KIND_WORDS.contains(&last) || upper.split('_').any(|word| ANYWHERE_WORDS.contains(&word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    /// Every credential variable the agent itself reads. A name added to the
    /// agent that this list does not catch is a credential its children
    /// would inherit.
    const AGENT_CREDENTIALS: &[&str] = &[
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GEMINI_API_KEY",
        "XAI_API_KEY",
        "DEEPSEEK_API_KEY",
        "MOONSHOT_API_KEY",
        "GLM_API_KEY",
        "DASHSCOPE_API_KEY",
        "GITHUB_TOKEN",
        "HF_TOKEN",
        "OPENCLAW_GATEWAY_TOKEN",
        "CLAUDE_CODE_SESSION_ACCESS_TOKEN",
        "ARKAVO_KAS_TOKEN",
        "ARKAVO_MASTER_KEY",
        "JWT_SECRET",
        "ARKAVO_GITHUB_WEBHOOK_SECRET",
        "ARKAVO_GITHUB_APP_PRIVATE_KEY",
    ];

    #[spec("PENV-003")]
    #[test]
    fn every_credential_the_agent_reads_is_secret_shaped() {
        for name in AGENT_CREDENTIALS {
            assert!(is_secret_name(name), "{name} would reach child processes");
        }
    }

    #[spec("PENV-003")]
    #[test]
    fn common_third_party_credentials_are_secret_shaped() {
        for name in [
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "NPM_CONFIG__AUTH",
            "CARGO_REGISTRY_TOKEN",
            "DB_PASSWORD_FILE",
            "github_pat",
        ] {
            assert!(is_secret_name(name), "{name}");
        }
    }

    #[spec("PENV-003")]
    #[test]
    fn toolchain_settings_are_not_secret_shaped() {
        for name in [
            "PATH",
            "HOME",
            "RUSTC_WRAPPER",
            "RUSTFLAGS",
            "CARGO_HOME",
            "SSH_AUTH_SOCK",
            "ARKAVO_MAX_TOKENS",
            "SCCACHE_GHA_ENABLED",
            "DATABASE_URL",
        ] {
            assert!(!is_secret_name(name), "{name}");
        }
    }
}
