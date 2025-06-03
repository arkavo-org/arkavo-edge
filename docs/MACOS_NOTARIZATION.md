# macOS Notarization Setup

This document describes the notarization process for macOS binaries in the Arkavo Edge release workflow.

## Overview

Apple requires all software distributed outside the Mac App Store to be notarized. Notarization is a process where Apple scans your software for malicious content and issues a ticket that allows it to run on macOS without Gatekeeper warnings.

## Prerequisites

- Active Apple Developer Program membership
- Developer ID Application certificate (already configured)
- Apple ID with app-specific password

## Required GitHub Secrets

The following secrets must be configured in your GitHub repository settings:

### APPLE_ID
- **Description**: Email address associated with your Apple Developer account
- **Example**: `developer@arkavo.com`
- **Where to find**: This is the email you use to sign in to developer.apple.com

### APPLE_APP_PASSWORD
- **Description**: App-specific password for notarization
- **How to create**:
  1. Sign in to https://appleid.apple.com
  2. Navigate to "Sign-In and Security"
  3. Find "App-Specific Passwords" section
  4. Click "+" or "Generate Password"
  5. Enter a label (e.g., "Arkavo Notarization")
  6. Copy the generated password (format: xxxx-xxxx-xxxx-xxxx)
- **Important**: This is NOT your regular Apple ID password

## Workflow Process

The release workflow performs these steps for macOS builds:

1. **Build**: Compiles the binary for aarch64-apple-darwin
2. **Strip**: Removes debug symbols to reduce size
3. **Codesign**: Signs the binary with Developer ID certificate
4. **Package**: Creates a ZIP file for notarization submission
5. **Notarize**: Submits to Apple's notarization service and waits for approval
6. **Staple**: Attaches the notarization ticket to the binary
7. **Release**: Creates the final tarball with the notarized binary

## Verification

After downloading a released binary, users can verify notarization:

```bash
# Check if binary is notarized
spctl -a -v arkavo

# Should output:
# arkavo: accepted
# source=Notarized Developer ID
```

## Troubleshooting

### Notarization Failures

If notarization fails, check:
- Certificate is valid and not expired
- Binary is properly signed with hardened runtime
- No unsigned libraries or frameworks included
- App-specific password is correct

### Common Issues

1. **"Unable to validate your application"**
   - Ensure the Developer ID certificate matches the Team ID

2. **"Package Invalid"**
   - Check that the ZIP contains the binary at the root level

3. **"Authentication failed"**
   - Verify APPLE_ID and APPLE_APP_PASSWORD secrets are correct
   - Ensure 2FA is enabled on your Apple ID

## Alternative: API Key Authentication

For enhanced security and reliability, consider using App Store Connect API keys instead of app-specific passwords:

1. Create an API key in App Store Connect
2. Add these secrets instead:
   - `APPLE_API_KEY`: Base64-encoded .p8 key file
   - `APPLE_KEY_ID`: Key ID from App Store Connect
   - `APPLE_ISSUER_ID`: Issuer ID from App Store Connect

This method doesn't require 2FA and is better suited for CI/CD environments.