# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Important Project Files
When starting a new conversation or initializing, please read these files:
- `README.md` - Project overview and basic usage
- `CLAUDE.md` - Coding guidelines (this file)
- `REQUIREMENTS.md` - Technical and functional requirements

## Build/Test Commands

### Core Library
- Build: `cargo build`
- Build all features: `cargo build --all-features`
- Format code: `cargo fmt --all`
- Check formatting: `cargo fmt --all --check`
- Run all tests: `cargo test`
- Run tests with all features: `cargo test --all-features`
- Run a single test: `cargo test test_name`
- Run tests with output: `cargo test -- --nocapture`
- Run clippy lints: `cargo clippy`
- Run clippy with all features: `cargo clippy --all-targets --all-features`
- Fix all clippy warnings: `cargo clippy --fix`
- Build with warnings as errors: `RUSTFLAGS="-D warnings" cargo build`
- Run clippy with warnings as errors: `cargo clippy --all-targets --all-features -- -D warnings`

### WASM
- Build WASM (web): `wasm-pack build --target web --out-dir pkg-web` (from `crates/wasm/`)
- Build WASM (bundler): `wasm-pack build --target bundler --out-dir pkg` (from `crates/wasm/`)
- Build WASM (nodejs): `wasm-pack build --target nodejs --out-dir pkg-node` (from `crates/wasm/`)
- Test WASM builds for all targets before committing

## Architecture Overview

### Workspace Structure
This is a Cargo workspace with the following crates:
- **`opentdf`**: Main library (root crate)
- **`opentdf-protocol`**: Protocol types and structures (no I/O, pure data)
- **`opentdf-crypto`**: Cryptographic operations (KEM, encryption, hashing)
- **`opentdf-wasm`**: WebAssembly bindings for browser use

### Core Modules
- **`src/lib.rs`**: Main library entry point, exports public API
- **`src/archive.rs`**: TDF archive creation and reading using ZIP format
  - `TdfArchive`: Read TDF archives from files or streams
  - `TdfArchiveBuilder`: Create new TDF archives with manifest and payload
- **`src/kas.rs`**: KAS (Key Access Service) client for key rewrap protocol
  - `KasClient`: Async HTTP client using reqwest + tokio
  - JWT signing with RS256 for authentication
  - Full KAS v2 rewrap protocol implementation
- **`src/manifest.rs`**: TDF manifest structure and serialization
  - `TdfManifest`: JSON manifest containing encryption metadata and policy
- **`src/policy.rs`**: Attribute-Based Access Control (ABAC) policy system
  - `AttributeIdentifier`: Namespace-qualified attribute names
  - `AttributeValue`: Type-safe attribute values (string, number, boolean, datetime, arrays)
  - `Operator`: Rich comparison operators (equals, contains, in, minimumOf, etc.)
  - `AttributePolicy`: Logical policy expressions with AND/OR/NOT
  - `Policy`: Complete policy with time constraints and dissemination

### Protocol Crate (`crates/protocol`)
- Pure data structures, no I/O operations
- Shared types between native and WASM
- KAS protocol request/response types
- WASM-compatible (uses `uuid` with "js" feature on wasm32)

### Crypto Crate (`crates/crypto`)
- AES-256-GCM encryption/decryption
- RSA-OAEP key encapsulation (SHA-1 for Go SDK compatibility, SHA-256 recommended)
- HMAC-SHA256 for policy binding
- Modular KEM traits for extensibility

### WASM Crate (`crates/wasm`)
- Browser-compatible bindings using wasm-bindgen
- Uses `opentdf` with `default-features = false` (no tokio)
- Implements KAS client using web_sys Fetch API
- Shares protocol structs with native implementation
- Supports all TDF operations in-browser (encrypt, decrypt, policy evaluation)

### Key Design Patterns
- **Encryption Flow**: Data → AES-256-GCM encryption → Policy binding via HMAC-SHA256 → ZIP archive
- **Policy Evaluation**: User attributes → Policy tree evaluation → Access decision with audit trail
- **Type Safety**: Strong typing with `thiserror` for error handling, `serde` for serialization

## Code Style Guidelines

- **Formatting**: Use `rustfmt` defaults (enforced via `cargo fmt`)
- **Error Handling**: Use `thiserror` for custom error types (see `PolicyError`, `EncryptionError`, `TdfError`)
- **Naming**: Follow Rust conventions - snake_case for variables/functions, CamelCase for types
- **Imports**: Group std imports first, then external crates, then local modules
- **Types**: Use strong typing and prefer specific error types over `Box<dyn Error>`
- **Documentation**: Document public API with doc comments
- **Testing**: Write unit tests for each module, integration tests for API
- **Error Types**: Create enum-based error types that implement `std::error::Error`
- **Serialization**: Use `serde` attributes consistently for JSON serialization
- **Security**: Never commit secrets, validate cryptographic operations
- **Commit Preparation**: Always run `cargo build`, `cargo clippy`, and `cargo fmt` before committing
- **Dead Code**: Remove unused code rather than using `#[allow(dead_code)]` attributes
- **Warnings**: Eliminate all compiler warnings before committing code

## Workflow Guidelines

- Always run the full test suite before submitting PRs: `cargo test`
- Address all clippy warnings before submitting code: `cargo clippy`
- Format code before committing: `cargo fmt`
- When adding new dependencies, document their purpose in comments
- Verify backwards compatibility when modifying public APIs
- Fix all compiler warnings before committing

## PR Review Process

- Use GitHub CLI (`gh`) for PR reviews: `gh pr review [PR-NUMBER]`
- PRs should be reviewed across multiple perspectives:
  - **Product**: Business value, user experience, and strategic alignment
  - **Development**: Code quality, performance, and adherence to standards
  - **Quality**: Test coverage, edge cases, and regression risks
  - **Security**: Vulnerabilities, data handling, and compliance
  - **DevOps**: CI/CD integration, configuration, and monitoring
  - **Usability**: API design and interaction patterns
- All PR reviews should identify specific issues that need to be fixed before merging
- PRs should include tests and documentation for new features
- No PR should be merged with pending warnings or clippy issues

## Pre-Commit Checklist

**IMPORTANT**: Run ALL of these commands before committing. CI will fail if any are skipped.

1. **Format code**: `cargo fmt --all`
   - Check formatting: `cargo fmt --all --check`

2. **Run clippy**: `cargo clippy --all-targets --all-features -- -D warnings`
   - Fix any warnings that appear
   - Re-run until no warnings remain

3. **Run all tests**: `cargo test --all-features`
   - Verify all tests pass
   - Fix any failing tests

4. **Test WASM builds** (if WASM code changed):
   ```bash
   cd crates/wasm
   wasm-pack build --target web --out-dir pkg-web
   wasm-pack build --target bundler --out-dir pkg
   wasm-pack build --target nodejs --out-dir pkg-node
   ```

5. **Build with warnings as errors**: `RUSTFLAGS="-D warnings" cargo build --all-features`
   - Ensures no warnings slip through

6. **Documentation and review**:
   - Ensure all new public APIs are documented
   - Verify security-sensitive code has been reviewed
   - Update CLAUDE.md if architecture changes

### Common Issues

- **UUID in WASM**: Protocol crate needs `uuid` with "js" feature for wasm32 targets
- **KAS endpoint paths**: Client appends `/kas/v2/rewrap` to base URL, don't duplicate `/kas`
- **Policy UUIDs**: Must be exactly 36 characters (standard UUID format)
- **Serde field names**: Use `#[serde(rename = "camelCase")]` for JSON API compatibility
- **Test assertions**: Error messages may change, test error types not exact strings