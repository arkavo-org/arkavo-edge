# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.19.0-alpha1] - 2025-01-29

### Added
- Multi-provider LLM architecture with dynamic model selection (PR #101)
- Extended Blueprint DSL v1.2 with LLM provider configuration support
- Shared HTTP client infrastructure with rustls (no OpenSSL dependency)
- Provider adapters for OpenAI and Anthropic with streaming support
- Comprehensive error taxonomy with retryable/non-retryable classification
- AES-256-GCM credential encryption using ring crate
- Provider factory pattern for extensible LLM support
- Migration support from Blueprint v1.1 to v1.2

### Changed
- Updated workspace version from 0.18.0 to 0.19.0
- Enhanced auth manager with secure credential storage
- Improved error messages for missing API keys

### Security
- Implemented proper credential encryption with PBKDF2 key derivation
- Removed plaintext credential storage
- Added backward compatibility for legacy base64 credentials

## [0.18.0] - Previous release
- (Previous release notes here)