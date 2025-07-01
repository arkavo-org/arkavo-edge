# Arkavo Protocol Schemas

This directory contains JSON schemas for various aspects of the Arkavo protocol.

## Directory Structure

- `openrpc/` - OpenRPC schemas for the A2A transport protocol
- `config/` - JSON schemas for configuration files (ServerConfig, RateLimitConfig, etc.)
- `wire/v1/` - JSON schemas for wire protocol messages (PromiseRequest, PromiseResponse, etc.)

## Generating Schemas

Schemas are generated from the Rust code using schemars. To regenerate:

```bash
# Generate all schemas
cargo xtask schema-gen

# Only generate config schemas
cargo xtask schema-gen --config

# Only generate wire protocol schemas  
cargo xtask schema-gen --wire

# Check if schemas are up to date (CI mode)
cargo xtask schema-gen --check
```

## CI Validation

The CI pipeline validates:

1. **Schema Structure** - All schemas conform to their respective meta-schemas
2. **Code vs Spec Drift** - Generated schemas match committed schemas
3. **Backwards Compatibility** - No breaking changes in pull requests
4. **Documentation Snippets** - JSON examples in docs are valid

## Using Schemas

### For Configuration Validation

```bash
# Validate a config file against its schema
ajv validate -s schemas/config/ServerConfig.json -d my-config.json
```

### For API Development

The OpenRPC schema can be used to generate:
- Client libraries in various languages
- API documentation
- Mock servers
- Test cases

### For Fuzzing

Wire protocol schemas are used in fuzz testing to ensure:
- All valid messages can be parsed
- Parsed messages round-trip correctly
- Invalid messages are properly rejected