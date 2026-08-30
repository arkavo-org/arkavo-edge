use arkavo_identity::IdentitySession;

pub(crate) fn login_help() -> &'static str {
    "    login          Sign in with Arkavo Creator\n    logout         Clear the stored identity token"
}

pub async fn execute_login() -> Result<(), Box<dyn std::error::Error>> {
    let session = IdentitySession::new();
    let sub = session.login().await?;
    println!("logged in as {sub}");
    Ok(())
}

pub async fn execute_logout() -> Result<(), Box<dyn std::error::Error>> {
    let session = IdentitySession::new();
    session.logout().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_mentions_login_and_logout() {
        let help = login_help();
        assert!(
            help.contains("login"),
            "usage should mention login: {help:?}"
        );
        assert!(
            help.contains("logout"),
            "usage should mention logout: {help:?}"
        );
    }

    #[test]
    fn commands_mod_declares_login_module() {
        let src = include_str!("mod.rs");
        assert!(
            src.contains("pub mod login;"),
            "commands/mod.rs should declare the login module"
        );
    }
}
