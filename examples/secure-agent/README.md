# Secure Agent

Demonstrates preflight policy configuration for input moderation.

## What You'll Learn

- How to configure preflight policies in AGENTS.md
- Available policy features (PII detection, SQL injection, etc.)
- How blocked inputs are handled
- Custom regex patterns for policy rules

## Quick Start

```bash
./launch.sh
```

Then test with different inputs to see policies in action.

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
| `AGENTS.md` | Agent config with preflight policies |
| `tasks.json` | Test tasks showing blocked/allowed inputs |
| `launch.sh` | Start the agent |
| `stop.sh` | Stop the agent |

## Next Steps

- Add custom regex patterns for your use case
- Combine with other examples to add security to mesh agents
- See `../CONCEPTS.md` for policy evaluation details
