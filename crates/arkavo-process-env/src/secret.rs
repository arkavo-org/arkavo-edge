//! Recognising a credential by its variable name.
//!
//! Every credential the agent reads follows the `<SCOPE>_<KIND>` convention
//! providers and CI systems use (`OPENAI_API_KEY`, `GITHUB_TOKEN`,
//! `JWT_SECRET`, `ARKAVO_MASTER_KEY`), so the last underscore-separated word
//! names the kind. A trailing `_FILE` is ignored, because `GITHUB_TOKEN_FILE`
//! points at the same credential. A word that contains `SECRET` or
//! `PASSWORD`, or is `TOKEN` or ends in it, marks the name wherever it
//! appears (`AWS_SECRET_ACCESS_KEY`, `PGPASSWORD`). `TOKENS` is a count, not
//! a credential.
//!
//! This is a deny-list and fails open twice. A credential stored under an
//! unconventional name passes, and so does one the operator chose the name
//! of (a provider's `auth_ref`); callers withhold those explicitly with
//! `ChildEnv::toolchain_withholding`. Names that are neither
//! credential-shaped nor declared or configured may therefore still reach a
//! toolchain child until a broker holds the credentials. See
//! `ChildEnv::toolchain` for why the toolchain profile accepts that.

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
    "PWD",
    "PASS",
    "JWT",
    "CWT",
    "BEARER",
    "COOKIE",
    "SESSION",
];

/// Whether an environment variable name, by convention, holds a
/// credential.
pub fn is_secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    // A shell keeps the working directory in a bare `PWD`; only a scoped
    // `MYSQL_PWD` is a password.
    if upper == "PWD" {
        return false;
    }
    let stem = upper.strip_suffix("_FILE").unwrap_or(&upper);
    let last = stem.rsplit('_').next().unwrap_or_default();
    KIND_WORDS.contains(&last)
        || stem.split('_').any(|word| {
            word.contains("SECRET") || word.contains("PASSWORD") || word.ends_with("TOKEN")
        })
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
    fn credentials_without_the_scope_kind_shape_are_secret_shaped() {
        for name in [
            "PGPASSWORD",
            "MYSQL_PWD",
            "DB_PASS",
            "ID_JWT",
            "ARKAVO_IDENTITY_CWT",
            "AUTH_BEARER",
            "SESSION_COOKIE",
            "GITHUB_TOKEN_FILE",
            "SSH_KEY_FILE",
            "GHTOKEN",
        ] {
            assert!(is_secret_name(name), "{name}");
        }
    }

    #[spec("PENV-003")]
    #[test]
    fn names_without_a_credential_word_are_not_secret_shaped() {
        for name in ["RUST_LOG", "CARGO_HOME", "PATH", "PWD", "OLDPWD", "LANG"] {
            assert!(!is_secret_name(name), "{name}");
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
