//! What `shell_exec` refuses outright (`AutoBlocked`): destructive, privilege,
//! system-control and remote-execution patterns, and shell metacharacters that
//! chain or substitute commands the segment allowlist cannot reason about.

/// Check if command matches the blocklist
pub(super) fn check_blocklist(cmd_lower: &str) -> Option<String> {
    // Destructive commands
    if cmd_lower.contains("rm -rf") || cmd_lower.contains("rm -r /") {
        return Some("Recursive delete blocked".to_string());
    }
    if cmd_lower.starts_with("rmdir") && cmd_lower.contains('/') {
        return Some("Directory removal with path blocked".to_string());
    }
    if cmd_lower.contains("mkfs") || cmd_lower.contains("format ") {
        return Some("Filesystem format blocked".to_string());
    }
    if cmd_lower.starts_with("dd if=") || cmd_lower.contains(" dd if=") {
        return Some("Raw disk write blocked".to_string());
    }

    // Privilege escalation
    if cmd_lower.starts_with("sudo ") || cmd_lower.contains(" sudo ") {
        return Some("Privilege escalation (sudo) blocked".to_string());
    }
    if cmd_lower.starts_with("su ") || cmd_lower.starts_with("su\n") {
        return Some("Privilege escalation (su) blocked".to_string());
    }
    if cmd_lower.contains("chmod 777") || cmd_lower.contains("chmod -R 777") {
        return Some("Overly permissive chmod blocked".to_string());
    }
    if cmd_lower.starts_with("chown ") && cmd_lower.contains(" /") {
        return Some("System ownership change blocked".to_string());
    }

    // System control
    let system_commands = ["shutdown", "reboot", "poweroff", "halt", "init "];
    for sc in system_commands {
        if cmd_lower.starts_with(sc) || cmd_lower.contains(&format!(" {}", sc)) {
            return Some(format!("System control command '{}' blocked", sc.trim()));
        }
    }
    if cmd_lower.starts_with("systemctl ") || cmd_lower.starts_with("launchctl ") {
        return Some("Service manager command blocked".to_string());
    }

    // Network exfiltration patterns
    if (cmd_lower.contains("curl ") || cmd_lower.contains("wget "))
        && (cmd_lower.contains(" | bash")
            || cmd_lower.contains(" | sh")
            || cmd_lower.contains("|bash")
            || cmd_lower.contains("|sh"))
    {
        return Some("Remote code execution pattern blocked".to_string());
    }

    // Fork bomb pattern
    if cmd_lower.contains(":(){ :|:& };:") || cmd_lower.contains(":(){:|:&};:") {
        return Some("Fork bomb blocked".to_string());
    }

    None
}

/// Shell metacharacters that split or rewrite the command in ways the segment
/// allowlist cannot reason about. Extends the original with a background-`&`
/// scan (that is not a `>&`/`&>`/`2>&1` fd-dup) and C0 control characters.
pub(super) fn check_injection(cmd: &str) -> Option<String> {
    if cmd.contains(';') {
        return Some("Command chaining (;) detected".to_string());
    }
    if cmd.contains("&&") {
        return Some("Command chaining (&&) detected".to_string());
    }
    if cmd.contains("||") {
        return Some("Command chaining (||) detected".to_string());
    }
    if cmd.contains('`') {
        return Some("Command substitution (backticks) detected".to_string());
    }
    if cmd.contains("$(") {
        return Some("Command substitution ($()) detected".to_string());
    }
    if cmd.contains("<(") || cmd.contains(">(") {
        return Some("Process substitution detected".to_string());
    }
    // Any C0 control char other than tab is a splitter (covers \n and \r).
    if cmd.chars().any(|c| c.is_control() && c != '\t') {
        return Some("Control character injection detected".to_string());
    }
    if has_background_amp(cmd) {
        return Some("Background/chaining (&) detected".to_string());
    }
    None
}

/// A `&` that is a job-control/chaining operator rather than part of a `>&`,
/// `&>` or `2>&1` file-descriptor redirection.
fn has_background_amp(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            let next = bytes.get(i + 1).copied();
            if next == Some(b'&') {
                i += 2; // `&&` handled by check_injection already; skip
                continue;
            }
            let prev = cmd[..i].trim_end().bytes().last();
            let is_fd_dup = next == Some(b'>') || prev == Some(b'>');
            if !is_fd_dup {
                return true;
            }
        }
        i += 1;
    }
    false
}
