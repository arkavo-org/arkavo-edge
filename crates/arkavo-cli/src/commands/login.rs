use arkavo_identity::IdentitySession;

pub(crate) fn login_help() -> &'static str {
    "    login          Sign in with Arkavo Creator\n    logout         Clear the stored identity token"
}

/// Which of the two identity commands was invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionCommand {
    Login,
    Logout,
}

impl SessionCommand {
    fn name(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Logout => "logout",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::Login => {
                "Sign in with Arkavo Creator\n\nUSAGE:\n    arkavo login\n\nOpens Arkavo Creator to approve the sign-in and stores the identity token\nfor later commands.\n\nOPTIONS:\n    -h, --help    Show this help"
            }
            Self::Logout => {
                "Clear the stored identity token\n\nUSAGE:\n    arkavo logout\n\nOPTIONS:\n    -h, --help    Show this help"
            }
        }
    }
}

/// What an invocation asks for, decided from its arguments alone.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SessionAction {
    Run,
    Help,
}

/// Decide what `login`/`logout` should do before any of it happens.
///
/// Both commands take no arguments, and both are irreversible from the
/// user's side (one starts a sign-in ceremony, the other deletes the token),
/// so anything unrecognized is refused rather than ignored: `logout --help`
/// used to delete the token it was asked to explain.
pub(crate) fn plan(command: SessionCommand, args: &[String]) -> Result<SessionAction, String> {
    let mut action = SessionAction::Run;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => action = SessionAction::Help,
            other => {
                return Err(format!(
                    "unexpected argument '{other}' for 'arkavo {name}'\nRun 'arkavo {name} --help' for usage",
                    name = command.name()
                ));
            }
        }
    }
    Ok(action)
}

// Both `Handle::block_on` and `Runtime::block_on` are disallowed by
// `.clippy.toml` (nesting a runtime inside a runtime can panic). This is a
// CLI entry point: it reuses an ambient runtime when one exists and only
// builds its own otherwise.
#[allow(clippy::disallowed_methods)]
pub fn execute(command: &str, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let command = match command {
        "login" => SessionCommand::Login,
        "logout" => SessionCommand::Logout,
        other => return Err(format!("'{other}' is not an identity command").into()),
    };
    match plan(command, args)? {
        SessionAction::Help => {
            println!("{}", command.help());
            Ok(())
        }
        SessionAction::Run => {
            let run_async = async {
                match command {
                    SessionCommand::Login => execute_login().await,
                    SessionCommand::Logout => execute_logout().await,
                }
            };
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => handle.block_on(run_async),
                Err(_) => {
                    let runtime = tokio::runtime::Runtime::new()?;
                    runtime.block_on(run_async)
                }
            }
        }
    }
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

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

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

    #[test]
    fn no_arguments_runs_the_command() {
        for command in [SessionCommand::Login, SessionCommand::Logout] {
            assert_eq!(plan(command, &[]), Ok(SessionAction::Run));
        }
    }

    /// Regression: `login --help` started a sign-in and `logout --help`
    /// deleted the stored token, because the arguments were never read.
    #[test]
    fn help_flags_ask_for_help_instead_of_running() {
        for command in [SessionCommand::Login, SessionCommand::Logout] {
            for flag in ["-h", "--help"] {
                assert_eq!(
                    plan(command, &args(&[flag])),
                    Ok(SessionAction::Help),
                    "{} {flag}",
                    command.name()
                );
            }
        }
    }

    #[test]
    fn unexpected_arguments_are_refused_before_anything_runs() {
        for command in [SessionCommand::Login, SessionCommand::Logout] {
            for unexpected in [
                args(&["--force"]),
                args(&["now"]),
                args(&["--help", "extra"]),
                args(&["extra", "--help"]),
                args(&["--verbose"]),
            ] {
                let err = plan(command, &unexpected).unwrap_err();
                assert!(
                    err.contains("unexpected argument"),
                    "{} {unexpected:?}: {err}",
                    command.name()
                );
                assert!(
                    err.contains(&format!("arkavo {} --help", command.name())),
                    "error should point at the command's help: {err}"
                );
            }
        }
    }

    #[test]
    fn each_command_describes_itself() {
        assert!(SessionCommand::Login.help().contains("arkavo login"));
        assert!(SessionCommand::Logout.help().contains("arkavo logout"));
    }
}
