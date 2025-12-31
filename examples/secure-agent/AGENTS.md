---
name: secure-agent
purpose: "Demonstrates preflight policy configuration for input moderation"
model: ministral-3b

# Preflight Moderation Policies
#
# Policies are evaluated before LLM inference using TØR-G boolean circuits.
# Each policy defines features to detect and an action (block/allow).
#
# Available features:
#   - InputContainsPII           : SSN, credit cards, emails
#   - InputContainsProfanity     : Toxicity keywords
#   - InputContainsSQLKeywords   : SELECT, DROP, INSERT, UPDATE, DELETE
#   - InputContainsShellCommands : rm, sudo, chmod, curl, wget
#   - InputContainsCodeBlock     : Triple backticks (```)
#   - InputContainsURL           : http:// or https:// links
#   - InputContainsBase64        : Base64-encoded data patterns
#   - InputLengthExceedsThreshold(N) : Input longer than N characters
#   - Custom(regex)              : Custom regex pattern

preflight:
  policies:
    # Block personally identifiable information
    - id: block_pii
      features:
        - InputContainsPII
      action: block
      description: "Blocks SSN, credit card numbers, and email addresses"
      enabled: true

    # Block SQL injection attempts
    - id: block_sql_injection
      features:
        - InputContainsSQLKeywords
      action: block
      description: "Blocks SQL keywords like DROP, SELECT, DELETE"
      enabled: true

    # Block shell command injection
    - id: block_shell_commands
      features:
        - InputContainsShellCommands
      action: block
      description: "Blocks shell commands like sudo, rm, chmod"
      enabled: true

    # Block excessively long inputs (100KB limit)
    - id: block_long_input
      features:
        - InputLengthExceedsThreshold(100000)
      action: block
      description: "Blocks inputs exceeding 100KB"
      enabled: true

# A2A Protocol Configuration
a2a:
  enabled: true
  discovery:
    mdns: true
---

# Secure Agent

This agent demonstrates how to configure preflight moderation policies
in AGENTS.md to protect against malicious inputs.

## Usage

Run the agent from this directory:

```bash
arkavo agent run
```

## Testing Policies

Try sending inputs that should be blocked:

```bash
# PII (SSN) - should be blocked
arkavo chat --prompt "My SSN is 123-45-6789"

# SQL injection - should be blocked
arkavo chat --prompt "DROP TABLE users"

# Shell commands - should be blocked
arkavo chat --prompt "sudo rm -rf /"

# Clean input - should pass
arkavo chat --prompt "What is the weather today?"
```

## Customizing Policies

Edit the `preflight.policies` section above to:
- Add new policies
- Disable policies by setting `enabled: false`
- Change features or actions
- Add custom regex patterns with `Custom(pattern)`
