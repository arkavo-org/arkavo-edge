# OpenTDF WASM

WebAssembly bindings for OpenTDF (Trusted Data Format), enabling data-centric security in browser environments.

## Features

- Create TDF archives with encrypted data
- Full KAS (Key Access Service) rewrap protocol support
- Decrypt TDF archives with KAS authorization
- Read TDF archives and access manifests
- Attribute-Based Access Control (ABAC) policy evaluation
- Browser-native implementation (ES Modules)
- Zero-copy operations where possible

## Installation

### From GitHub Packages (Recommended)

The package is published to GitHub Packages. You'll need a GitHub account and a personal access token.

**Step 1:** Create a GitHub [Personal Access Token](https://github.com/settings/tokens) with `read:packages` permission.

**Step 2:** Configure npm to use GitHub Packages. Create/update `~/.npmrc`:

```
@arkavo-org:registry=https://npm.pkg.github.com
//npm.pkg.github.com/:_authToken=YOUR_GITHUB_TOKEN
```

**Step 3:** Install the package:

```bash
npm install @arkavo-org/opentdf-wasm
```

See [NPM_PUBLISHING.md](NPM_PUBLISHING.md) for detailed installation instructions, CI/CD setup, and troubleshooting.

### From GitHub Releases

Download pre-built WASM binaries without npm:

```bash
# Download and extract
wget https://github.com/arkavo-org/opentdf-rs/releases/latest/download/opentdf-wasm-web.tar.gz
tar -xzf opentdf-wasm-web.tar.gz

# Contains web build ready to use
```

## Usage

### Browser

```javascript
import init, { tdf_create, tdf_read, tdf_decrypt_with_kas, access_evaluate, version } from '@arkavo-org/opentdf-wasm';

// Initialize the WASM module
await init();

console.log('OpenTDF WASM version:', version());

// Obtain OAuth token (example - your implementation will vary)
const oauthToken = await getOAuthToken();

// Create a TDF
const data = btoa('Sensitive information'); // Base64 encode
const policy = {
  uuid: crypto.randomUUID(),
  body: {
    attributes: [],
    dissem: ['user@example.com']
  }
};

const result = await tdf_create(
  data,
  'https://kas.example.com/kas',
  JSON.stringify(policy)
);

if (result.success) {
  const tdfArchive = result.data; // Base64-encoded TDF
  console.log('TDF created:', tdfArchive);

  // Read the TDF manifest
  const manifestResult = tdf_read(tdfArchive);
  if (manifestResult.success) {
    const manifest = JSON.parse(manifestResult.data);
    console.log('Manifest:', manifest);
  }

  // Decrypt the TDF using KAS
  const decryptResult = await tdf_decrypt_with_kas(tdfArchive, oauthToken);
  if (decryptResult.success) {
    const plaintext = atob(decryptResult.data); // Base64 decode
    console.log('Decrypted:', plaintext);
  } else {
    console.error('Decryption error:', decryptResult.error);
  }
} else {
  console.error('Error:', result.error);
}
```

### Attribute-Based Access Control (ABAC)

```javascript
// Define an attribute policy
const policy = {
  attribute: {
    namespace: "gov.example",
    name: "clearance"
  },
  operator: "minimumOf",
  value: "SECRET"
};

// User attributes
const userAttributes = {
  "gov.example:clearance": "TOP_SECRET",
  "gov.example:department": "ENGINEERING"
};

// Evaluate access
const accessResult = access_evaluate(
  JSON.stringify(policy),
  JSON.stringify(userAttributes)
);

if (accessResult.success) {
  const granted = accessResult.data === 'true';
  console.log('Access granted:', granted);
} else {
  console.error('Evaluation error:', accessResult.error);
}
```

### Complex ABAC Policies

```javascript
// Create a policy requiring both clearance and department
const complexPolicy = {
  type: "AND",
  conditions: [
    {
      attribute: {
        namespace: "gov.example",
        name: "clearance"
      },
      operator: "minimumOf",
      value: "SECRET"
    },
    {
      attribute: {
        namespace: "gov.example",
        name: "department"
      },
      operator: "in",
      value: ["ENGINEERING", "EXECUTIVE"]
    }
  ]
};

// Create TDF with ABAC policy
const fullPolicy = {
  uuid: crypto.randomUUID(),
  body: {
    attributes: [complexPolicy],
    dissem: ['user@example.com']
  }
};

const result = tdf_create(
  btoa('Classified data'),
  'https://kas.example.com',
  JSON.stringify(fullPolicy)
);
```

## API Reference

### `version(): string`

Returns the version of the OpenTDF WASM library.

### `tdf_create(data: string, kas_url: string, policy_json: string): WasmResult`

Creates a TDF archive with encrypted data.

**Parameters:**
- `data`: Base64-encoded data to encrypt
- `kas_url`: KAS (Key Access Service) URL
- `policy_json`: JSON string containing the policy

**Returns:** `WasmResult` with base64-encoded TDF archive in `data` field

### `tdf_read(tdf_data: string): WasmResult`

Reads a TDF archive and returns its manifest.

**Parameters:**
- `tdf_data`: Base64-encoded TDF archive

**Returns:** `WasmResult` with JSON manifest in `data` field

### `tdf_decrypt_with_kas(tdf_data: string, kas_token: string): Promise<WasmResult>`

Decrypts a TDF archive using the KAS rewrap protocol (async).

**Parameters:**
- `tdf_data`: Base64-encoded TDF archive
- `kas_token`: OAuth bearer token for KAS authentication

**Returns:** Promise resolving to `WasmResult` with base64-encoded plaintext in `data` field

**Flow:**
1. Parses TDF manifest and extracts policy/key access info
2. Generates ephemeral RSA-2048 key pair
3. Builds and signs JWT rewrap request (ES256)
4. POSTs to KAS `/v2/rewrap` endpoint with OAuth token
5. Unwraps returned key using RSA-OAEP (SHA-1)
6. Decrypts payload with AES-256-GCM

**Error Handling:**
- 401: Invalid OAuth token
- 403: Access denied (policy evaluation failed)
- Network errors: CORS or connectivity issues

### `policy_create(policy_json: string): WasmResult`

Creates and validates a policy from JSON.

**Parameters:**
- `policy_json`: JSON string containing policy definition

**Returns:** `WasmResult` with validated policy JSON in `data` field

### `access_evaluate(policy_json: string, attributes_json: string): WasmResult`

Evaluates an attribute-based access control policy.

**Parameters:**
- `policy_json`: JSON string containing the attribute policy
- `attributes_json`: JSON string containing user attributes as key-value pairs

**Returns:** `WasmResult` with boolean string ("true"/"false") in `data` field

### `attribute_identifier_create(identifier: string): WasmResult`

Creates an attribute identifier from namespace:name format.

**Parameters:**
- `identifier`: String in format "namespace:name"

**Returns:** `WasmResult` with JSON attribute identifier in `data` field

### `WasmResult`

All functions return a `WasmResult` object with the following properties:

- `success: boolean` - Whether the operation succeeded
- `data: string | null` - Result data if successful
- `error: string | null` - Error message if failed

## Building from Source

### Prerequisites

- Rust 1.70 or later
- wasm-pack (`cargo install wasm-pack`)

### Build

```bash
cd crates/wasm
wasm-pack build --target web --out-dir pkg-web
# or
npm run build
```

## Testing

```bash
wasm-pack test --headless --firefox
```

## Size Optimization

The WASM binary is optimized for size using:
- `opt-level = "z"` - Optimize for size
- `lto = true` - Link-time optimization
- `codegen-units = 1` - Single codegen unit for better optimization
- `wasm-opt` with `-O3` - Additional WebAssembly-specific optimization

Typical bundle size:
- ~730 KB uncompressed, ~230 KB gzipped

The bundle includes full KAS rewrap protocol with:
- RSA-2048 key generation and OAEP encryption/decryption
- P-256 ECDSA JWT signing (ES256)
- AES-256-GCM encryption/decryption
- Browser Fetch API integration

## Browser Compatibility

- Chrome/Edge: 84+
- Firefox: 79+
- Safari: 15+
- All browsers with WebAssembly support

## KAS Integration

The WASM module includes **full KAS (Key Access Service) integration** for both encryption and decryption:

### Encryption (tdf_create)
- Automatically fetches KAS public key via browser Fetch API
- Wraps DEK with RSA-2048-OAEP (SHA-1 for Go SDK compatibility)
- Creates TDF with proper policy binding
- DEK never leaves WASM environment

### Decryption (tdf_decrypt_with_kas)
- Generates ephemeral RSA-2048 key pair
- Signs JWT rewrap request with P-256 ECDSA (ES256)
- POSTs to KAS `/v2/rewrap` endpoint with OAuth token
- Unwraps returned key with RSA-OAEP
- Decrypts payload with AES-256-GCM
- All cryptographic operations happen in WASM

### OAuth Token Management
The WASM module requires an OAuth bearer token for KAS operations. Your application must:
1. Obtain OAuth token from your identity provider
2. Pass token to `tdf_create()` and `tdf_decrypt_with_kas()`
3. Handle token refresh when needed

### CORS Configuration
KAS must be configured to allow CORS requests from your domain:
```
Access-Control-Allow-Origin: https://your-domain.com
Access-Control-Allow-Methods: GET, POST, OPTIONS
Access-Control-Allow-Headers: Authorization, Content-Type, Accept
```

### Security Guarantees
✅ DEK never exposed in JavaScript
✅ Ephemeral keys generated fresh for each operation
✅ Full KAS authorization enforcement
✅ Compatible with OpenTDF Go/Python SDKs
✅ Proper zero-trust architecture

## Security Considerations

### WebCrypto Security Architecture

The WASM module delegates sensitive RSA operations to the browser's native WebCrypto API (SubtleCrypto), providing several security advantages:

#### Constant-Time Operations
- RSA-OAEP encryption/decryption is implemented by the browser's native code
- Browser implementations are designed to resist timing side-channel attacks
- No exposure to RUSTSEC-2023-0071 (timing vulnerability in RustCrypto RSA)

#### Hardware Acceleration
- Modern browsers utilize hardware acceleration (AES-NI, SHA extensions) where available
- Provides both performance benefits and additional protection against software timing attacks

#### Key Isolation
- WebCrypto `CryptoKey` objects are opaque - private key material cannot be extracted by JavaScript
- Ephemeral keys are generated fresh for each decrypt operation
- Key material is managed by the browser's secure key storage

### Browser Security Context vs Native Rust

| Aspect | Browser (WASM) | Native (Rust) |
|--------|---------------|---------------|
| RSA Backend | WebCrypto (SubtleCrypto) | aws-lc-rs |
| Timing Attack Resistance | Browser-native constant-time | aws-lc-rs constant-time |
| FIPS Validation | Browser-dependent | aws-lc-rs FIPS 140-3 |
| Memory Management | Browser GC | Explicit `zeroize` crate |
| Random Source | `crypto.getRandomValues()` | OS CSPRNG |
| Key Storage | CryptoKey objects | Memory (zeroized on drop) |

#### Memory Management Considerations

In WASM environments:
- JavaScript's garbage collector manages memory
- Sensitive data in JS strings/arrays may persist until GC runs
- WASM linear memory is not automatically cleared
- Best practice: minimize sensitive data exposure in JavaScript layer

In Native environments:
- All key types implement `Zeroize` and `ZeroizeOnDrop`
- Memory is explicitly cleared when keys go out of scope
- Compiler optimizations are prevented from eliding zeroization

### Security Properties

#### What This Implementation Provides

1. **Transport Security**
   - All KAS communication over HTTPS (enforced by browser)
   - CORS policies prevent unauthorized cross-origin access
   - OAuth bearer tokens for authentication

2. **Cryptographic Security**
   - AES-256-GCM for payload encryption (128-bit authentication tag)
   - RSA-2048-OAEP for key wrapping (SHA-1 for Go SDK compatibility)
   - HMAC-SHA256 for policy binding integrity
   - Ephemeral key pairs for each KAS interaction

3. **Key Management Security**
   - DEK (Data Encryption Key) never exposed to JavaScript
   - Ephemeral RSA keys generated fresh for each decrypt operation
   - WebCrypto keys are non-extractable where possible

4. **Integrity Protection**
   - Per-segment GMAC authentication tags
   - Root signature over all segment tags
   - Policy binding verification

#### What This Implementation Does NOT Provide

- Protection against malicious browser extensions
- Protection against compromised browser environments
- Hardware-backed key storage (HSM/TPM)
- Post-quantum cryptographic algorithms (future enhancement)

### Recommended Usage Patterns

```javascript
// DO: Let WASM handle sensitive operations
const result = await tdf_decrypt_with_kas(tdfData, oauthToken);

// DO: Clear sensitive data after use
let plaintext = atob(result.data);
processData(plaintext);
plaintext = null; // Allow GC to reclaim

// DON'T: Extract and store decryption keys in JavaScript
// DON'T: Log TDF contents or decrypted data
// DON'T: Store OAuth tokens in localStorage (use sessionStorage or memory)
```

### Compliance Considerations

#### Algorithm Support

| Standard | Support Level |
|----------|---------------|
| NIST SP 800-38D (AES-GCM) | Full |
| NIST SP 800-56B (RSA Key Transport) | Full (OAEP) |
| FIPS 186-5 (ECDSA) | Full (P-256) |
| OpenTDF Specification | Full |

#### WebCrypto Browser Compatibility

WebCrypto (SubtleCrypto) support required for all operations:
- Chrome 37+ (August 2014)
- Firefox 34+ (December 2014)
- Safari 11+ (September 2017)
- Edge 12+ (July 2015)

All supported browsers provide equivalent cryptographic security properties.

## License

Apache-2.0

## Contributing

See the main [OpenTDF-RS repository](https://github.com/arkavo-org/opentdf-rs) for contribution guidelines.
