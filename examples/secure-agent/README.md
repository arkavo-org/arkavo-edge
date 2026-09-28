# Secure Agent

<!-- ARKAVO-CAPABILITY: network-security -->
> **Specs**: [17 scenarios](../../specs/arkavo-edge/network-security.spec.yaml)
> **Browse**: `cargo xtask capabilities network-security`
<!-- /ARKAVO-CAPABILITY -->

Demonstrates preflight policy configuration for input moderation. Peers are
discovered via mDNS (`runtime.mdns: true` in the kit); A2A enablement itself
is implicit in running `arkavo agent` and isn't a separate kit field.

## What You'll Learn

- How to configure preflight policies in a SwarmKit kit
- Available policy features (PII detection, SQL injection, etc.)
- How blocked inputs are handled
- Custom regex patterns for policy rules

## Quick Start

To test policy enforcement, run the agent standalone:

```bash
./launch.sh
```

Then in another terminal, test with different inputs:

```bash
arkavo chat --prompt "My SSN is 123-45-6789"  # Should be blocked
arkavo chat --prompt "What is the weather?"   # Should pass
```

**Note:** The demo.sh runner uses generic agents without policies.
To see actual policy enforcement, run this agent standalone.

## Architecture

```
Input → Preflight Check → [Allow] → LLM → Response
                       → [Block] → Rejection
```

## Available Policy Features

| Feature | Detects |
|---------|---------|
| `InputContainsPII` | SSN, credit cards, emails |
| `InputContainsProfanity` | Toxicity keywords |
| `InputContainsSQLKeywords` | SELECT, DROP, INSERT, DELETE |
| `InputContainsShellCommands` | rm, sudo, chmod, curl, wget |
| `InputContainsCodeBlock` | Triple backticks |
| `InputContainsURL` | http:// or https:// links |
| `InputContainsBase64` | Base64-encoded data |
| `InputLengthExceedsThreshold(N)` | Inputs > N characters |
| `Custom(regex)` | Custom regex pattern |

## Configured Policies

This example includes four policies:

1. **block_pii** - Blocks SSN, credit card numbers, emails
2. **block_sql_injection** - Blocks SQL keywords
3. **block_shell_commands** - Blocks shell commands
4. **block_long_input** - Blocks inputs > 100KB

## Testing

```bash
# Should be BLOCKED (PII)
arkavo chat --prompt "My SSN is 123-45-6789"

# Should be BLOCKED (SQL injection)
arkavo chat --prompt "DROP TABLE users"

# Should be BLOCKED (shell command)
arkavo chat --prompt "sudo rm -rf /"

# Should be ALLOWED
arkavo chat --prompt "What is the weather today?"
```

## Files

| File | Purpose |
|------|---------|
| `secure-agent.swarmkit.yaml` | Agent config with preflight policies |
| `tasks.json` | Test tasks showing blocked/allowed inputs |
| `launch.sh` | Start the agent |
| `stop.sh` | Stop the agent |

## Next Steps

- Add custom regex patterns for your use case
- Combine with other examples to add security to mesh agents
- See `../CONCEPTS.md` for policy evaluation details
