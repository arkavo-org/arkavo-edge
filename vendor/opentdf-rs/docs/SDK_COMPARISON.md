# OpenTDF SDK Comparison Analysis

## Executive Summary

This document compares the Rust implementation (opentdf-rs) against the Swift SDK (OpenTDFKit) and Golang SDK from the OpenTDF platform, evaluating compliance with the OpenTDF specification.

**Analysis Date**: 2025-09-29
**Repositories Analyzed**:
- opentdf-rs: `/Users/paul/Projects/arkavo/opentdf-rs`
- OpenTDFKit (Swift): `/Users/paul/Projects/arkavo/OpenTDFKit`
- platform/sdk (Go): `/Users/paul/Projects/opentdf/platform/sdk`
- Spec: `https://github.com/opentdf/spec`

---

## Implementation Comparison Matrix

### 1. TDF Format Support

| Feature | opentdf-rs | OpenTDFKit | Go SDK | Spec Compliance |
|---------|-----------|------------|--------|-----------------|
| **Standard TDF (ZIP-based)** | ✅ Full | ✅ Full | ✅ Full | ✅ |
| **NanoTDF** | ❌ Not implemented | ✅ Full | ✅ Full | ⚠️ Missing |
| **Manifest Schema** | ✅ Implemented | ✅ Implemented | ✅ Implemented | ✅ |
| **Policy Object** | ✅ ABAC support | ✅ Basic | ✅ Full | ✅ |

### 2. Cryptographic Operations

| Feature | opentdf-rs | OpenTDFKit | Go SDK | Notes |
|---------|-----------|------------|--------|-------|
| **AES-256-GCM** | ✅ | ✅ | ✅ | All implement |
| **HMAC-SHA256** | ✅ Policy binding | ✅ Policy binding | ✅ Policy binding | Standard |
| **ECDH Key Exchange** | ❌ | ✅ secp256r1/384r1/521r1 | ✅ secp256r1 | NanoTDF requirement |
| **ECDSA Signatures** | ❌ | ✅ | ❌ | NanoTDF binding |
| **GMAC Binding** | ❌ | ✅ | ✅ | NanoTDF binding |
| **Key Wrapping** | ✅ AES-GCM | ✅ EC-based | ✅ Multiple methods | Different approaches |

### 3. Policy System

| Feature | opentdf-rs | OpenTDFKit | Go SDK | Spec Requirement |
|---------|-----------|------------|--------|------------------|
| **Attribute-Based Access Control (ABAC)** | ✅ Comprehensive | ⚠️ Basic | ✅ Full | ✅ Required |
| **Hierarchical Attributes** | ✅ | ❌ | ✅ | ⚠️ Optional |
| **Logical Operators (AND/OR/NOT)** | ✅ | ❌ | ✅ | ✅ Required |
| **Time-based Constraints** | ✅ | ❌ | ✅ | ⚠️ Optional |
| **Operator Support** | ✅ 14 operators | ⚠️ Limited | ✅ Full | ✅ |
| **Remote Policy** | ⚠️ Partial | ✅ | ✅ | ✅ Required |
| **Embedded Policy** | ✅ | ✅ | ✅ | ✅ Required |
| **Policy Binding Verification** | ✅ HMAC | ✅ HMAC/ECDSA | ✅ HMAC/GMAC | ✅ |

### 4. KAS Integration

| Feature | opentdf-rs | OpenTDFKit | Go SDK | Spec Requirement |
|---------|-----------|------------|--------|------------------|
| **KAS Rewrap Protocol** | ❌ | ✅ | ✅ | ✅ Required |
| **Public Key Distribution** | ❌ | ✅ KeyStore | ✅ | ✅ Required |
| **Metadata Handling** | ⚠️ Partial | ✅ | ✅ | ✅ Required |
| **Multi-KAS Support** | ❌ | ❌ | ✅ | ⚠️ Optional |

### 5. Archive Structure

| Feature | opentdf-rs | OpenTDFKit | Go SDK | Implementation |
|---------|-----------|------------|--------|----------------|
| **ZIP Archive Creation** | ✅ | ✅ | ✅ | `zip` crate / standard libs |
| **Manifest (0.manifest.json)** | ✅ | ✅ | ✅ | JSON serialization |
| **Payload (0.payload)** | ✅ | ✅ | ✅ | Encrypted binary |
| **Streaming Support** | ❌ | ✅ | ✅ | Memory efficiency |

### 6. Attribute System

| Operator | opentdf-rs | OpenTDFKit | Go SDK | Use Case |
|----------|-----------|------------|--------|----------|
| **Equals** | ✅ | ⚠️ Implicit | ✅ | Exact match |
| **NotEquals** | ✅ | ❌ | ✅ | Negation |
| **GreaterThan/LessThan** | ✅ | ❌ | ✅ | Numeric comparison |
| **Contains** | ✅ | ❌ | ✅ | String search |
| **In/NotIn** | ✅ | ⚠️ Array | ✅ | Set membership |
| **AllOf/AnyOf** | ✅ | ❌ | ✅ | Array operations |
| **MinimumOf/MaximumOf** | ✅ | ❌ | ✅ | Hierarchical levels |
| **Present/NotPresent** | ✅ | ❌ | ✅ | Existence check |

### 7. MCP (Model Context Protocol) Integration

| Feature | opentdf-rs | OpenTDFKit | Go SDK | Purpose |
|---------|-----------|------------|--------|---------|
| **MCP Server** | ✅ Dedicated crate | ❌ | ❌ | AI integration |
| **JSON-RPC 2.0** | ✅ | ❌ | ❌ | Protocol |
| **Tool Definitions** | ✅ 10+ tools | ❌ | ❌ | Capabilities |
| **Audit Logging** | ✅ Comprehensive | ❌ | ⚠️ Basic | Compliance |

### 8. Testing & Performance

| Aspect | opentdf-rs | OpenTDFKit | Go SDK |
|--------|-----------|------------|--------|
| **Unit Tests** | ✅ | ✅ | ✅ |
| **Integration Tests** | ✅ | ✅ | ✅ |
| **Benchmarks** | ❌ | ✅ Comprehensive | ⚠️ Some |
| **Fuzz Testing** | ❌ | ❌ | ✅ |
| **CLI Tool** | ❌ | ✅ | ⚠️ Examples |

---

## Key Findings

### Strengths of opentdf-rs

1. **Advanced ABAC Implementation**
   - Most comprehensive attribute policy system among all three SDKs
   - 14 different operators including hierarchical (MinimumOf/MaximumOf)
   - Full logical operator support (AND/OR/NOT with nesting)
   - Time-based constraints with validity periods

2. **MCP Server Integration**
   - First-class AI integration through Model Context Protocol
   - Unique capability not present in other SDKs
   - Comprehensive audit logging for compliance

3. **Type Safety**
   - Leverages Rust's type system for compile-time guarantees
   - Strong error handling with `thiserror`
   - Memory safety without garbage collection

4. **Modern Architecture**
   - Clean separation of concerns (crypto, archive, policy, manifest modules)
   - Well-structured error types per module
   - Good documentation in code

### Critical Gaps in opentdf-rs

1. **NanoTDF Support Missing** ⚠️ HIGH PRIORITY
   - Both Swift and Go SDKs have full NanoTDF implementation
   - NanoTDF is part of the official OpenTDF spec
   - Required for IoT/embedded/mobile use cases
   - Minimum overhead < 200 bytes vs standard TDF ~1KB+

2. **No KAS Rewrap Protocol** ⚠️ HIGH PRIORITY
   - Cannot interact with real KAS servers for key unwrapping
   - Limits practical deployment scenarios
   - Both other SDKs have full KAS integration
   - Required for production use

3. **Missing ECC Support** ⚠️ MEDIUM PRIORITY
   - No Elliptic Curve Cryptography implementation
   - Required for NanoTDF
   - Swift SDK supports: secp256r1, secp384r1, secp521r1
   - Go SDK supports: secp256r1

4. **No Streaming Support** ⚠️ MEDIUM PRIORITY
   - Loads entire payloads into memory
   - Go and Swift SDKs support streaming
   - Important for large file handling
   - Performance implications

5. **Limited CLI/Tooling** ⚠️ LOW PRIORITY
   - No command-line tool for encryption/decryption
   - OpenTDFKit has full-featured CLI with format detection
   - Useful for testing and adoption

---

## Architecture Comparison

### Manifest Structure

All three implementations follow the OpenTDF manifest schema closely:

**opentdf-rs** (`src/manifest.rs`):
```rust
pub struct TdfManifest {
    pub payload: Payload,
    pub encryption_information: EncryptionInformation,
}

pub struct EncryptionInformation {
    pub encryption_type: String,
    pub key_access: Vec<KeyAccess>,
    pub method: EncryptionMethod,
    pub integrity_information: IntegrityInformation,
    pub policy: String,
}
```

**OpenTDFKit** (`TDFManifest.swift`):
```swift
public struct TDFManifest: Codable, Sendable {
    public var schemaVersion: String
    public var payload: TDFPayloadDescriptor
    public var encryptionInformation: TDFEncryptionInformation
    public var assertions: [TDFAssertion]?
}
```

**Go SDK** (`manifest.go`):
```go
type Manifest struct {
    EncryptionInformation `json:"encryptionInformation"`
    Payload               `json:"payload"`
    Assertions            []Assertion `json:"assertions,omitempty"`
    TDFVersion            string      `json:"schemaVersion,omitempty"`
}
```

**Analysis**: All three are spec-compliant. Go SDK has most complete implementation with assertions support.

### Policy System Architecture

**opentdf-rs** - Most Advanced:
```rust
pub enum AttributePolicy {
    Condition(AttributeCondition),
    And(Vec<AttributePolicy>),
    Or(Vec<AttributePolicy>),
    Not(Box<AttributePolicy>),
}

// 14 operators including hierarchical
pub enum Operator {
    Equals, NotEquals, GreaterThan, LessThan,
    Contains, In, AllOf, AnyOf, NotIn,
    MinimumOf, MaximumOf,  // Hierarchical
    Present, NotPresent,
}
```

**OpenTDFKit** - Basic:
```swift
public struct Policy {
    public var type: PolicyType
    public var body: String?
    public var remote: ResourceLocator?
    public var binding: PolicyBinding?
}
// Limited policy evaluation capability
```

**Go SDK** - Comprehensive:
```go
type PolicyObject struct {
    UUID string `json:"uuid"`
    Body struct {
        DataAttributes []attributeObject `json:"dataAttributes"`
        Dissem         []string          `json:"dissem"`
    } `json:"body"`
}
// Full attribute namespace support via platform
```

---

## Specification Compliance Assessment

### Standard TDF Format

| Spec Requirement | opentdf-rs | Status |
|-----------------|-----------|--------|
| ZIP container format | ✅ | Compliant |
| 0.manifest.json structure | ✅ | Compliant |
| 0.payload encryption | ✅ | Compliant |
| AES-256-GCM encryption | ✅ | Compliant |
| HMAC-SHA256 policy binding | ✅ | Compliant |
| Key wrapping | ✅ | Compliant |
| Integrity information | ✅ | Compliant |

### NanoTDF Format

| Spec Requirement | opentdf-rs | Status |
|-----------------|-----------|--------|
| Magic number "L1L" | ❌ | Not Implemented |
| Header structure | ❌ | Not Implemented |
| ECC curves support | ❌ | Not Implemented |
| GMAC binding | ❌ | Not Implemented |
| ECDSA binding | ❌ | Not Implemented |
| 3-byte IV | ❌ | Not Implemented |
| Resource locators | ❌ | Not Implemented |

### Protocol Requirements

| Spec Requirement | opentdf-rs | Status |
|-----------------|-----------|--------|
| KAS public key retrieval | ❌ | Not Implemented |
| Rewrap protocol | ❌ | Not Implemented |
| Policy evaluation at KAS | ⚠️ | Local only |
| OAuth/OIDC integration | ❌ | Not Implemented |

---

## Recommendations

### Priority 1: Critical for Production Use

1. **Implement KAS Rewrap Protocol**
   - Add HTTP/gRPC client for KAS communication
   - Implement rewrap request/response handling
   - Add authentication support (OAuth2, mTLS)
   - Reference: Go SDK's `kas_client.go` is excellent

2. **Add NanoTDF Support**
   - Implement binary header parsing/serialization
   - Add ECC curve support (at minimum secp256r1)
   - Implement GMAC and ECDSA policy binding
   - Resource locator types
   - Reference: OpenTDFKit's `NanoTDF.swift` is very clean

### Priority 2: Important Enhancements

3. **Streaming Support**
   - Implement chunk-based encryption/decryption
   - Add streaming reader/writer traits
   - Memory-efficient large file handling
   - Reference: Go SDK's approach in `tdf.go`

4. **Complete KAS Integration**
   - Public key distribution
   - Key metadata handling
   - Multi-KAS support
   - Cached key management

### Priority 3: Nice to Have

5. **CLI Tool**
   - Command-line encrypt/decrypt operations
   - Format detection and conversion
   - Testing and demo purposes
   - Reference: OpenTDFKit's CLI is excellent

6. **Performance Benchmarks**
   - Encryption/decryption throughput
   - Memory usage profiling
   - Comparison with other SDKs
   - Reference: OpenTDFKit has comprehensive benchmarks

7. **Additional Testing**
   - Fuzz testing (like Go SDK)
   - Cross-SDK interoperability tests
   - Compatibility test suite

---

## Implementation Strategy

### Phase 1: Foundation (2-3 weeks)

**Goal**: Enable basic KAS integration

- [ ] Add `reqwest` or `tonic` for HTTP/gRPC
- [ ] Implement KAS client module
- [ ] Add rewrap protocol structs
- [ ] Basic OAuth2 authentication
- [ ] Integration tests with mock KAS

### Phase 2: NanoTDF Core (3-4 weeks)

**Goal**: Basic NanoTDF support

- [ ] Add `p256` or `k256` crate for ECC
- [ ] Implement header structure
- [ ] Binary parser/serializer
- [ ] ECDH key exchange
- [ ] GMAC policy binding
- [ ] NanoTDF reader/writer

### Phase 3: Enhancement (2-3 weeks)

**Goal**: Production-ready features

- [ ] Streaming support for large files
- [ ] Additional ECC curves (secp384r1, secp521r1)
- [ ] ECDSA signature support
- [ ] Resource locator types
- [ ] Encrypted policy support

### Phase 4: Tooling & Testing (1-2 weeks)

**Goal**: Developer experience

- [ ] CLI tool (`opentdf-cli` binary)
- [ ] Performance benchmarks
- [ ] Cross-SDK compatibility tests
- [ ] Documentation examples
- [ ] Quick start guide

---

## Code Examples from Other SDKs

### NanoTDF Creation (Swift)

```swift
// From OpenTDFKit
let kasRL = ResourceLocator(protocolEnum: .http, body: "kas.example.com")
let kasMetadata = KasMetadata(
    resourceLocator: kasRL!,
    publicKey: publicKey,
    curve: .secp256r1
)
let remotePolicy = ResourceLocator(
    protocolEnum: .sharedResourceDirectory,
    body: "policy-id"
)
var policy = Policy(
    type: .remote,
    body: nil,
    remote: remotePolicy,
    binding: nil
)
let nanoTDF = try createNanoTDF(
    kas: kasMetadata,
    policy: &policy,
    plaintext: data
)
```

### KAS Rewrap (Go)

```go
// From Go SDK
func (r *Reader) buildKey(ctx context.Context, results []kaoResult) error {
    // Create rewrap request
    req := &kas.RewrapRequest{
        Policy: r.manifest.EncryptionInformation.Policy,
        Entity: entityObject,
        KeyAccess: keyAccessObject,
    }

    // Call KAS
    resp, err := kasClient.Rewrap(ctx, req)
    if err != nil {
        return err
    }

    // Unwrap the key
    r.payloadKey = unwrapKey(resp.Metadata)
    return nil
}
```

---

## Appendix: File Structure Comparison

### opentdf-rs
```
src/
├── lib.rs          # Public API
├── archive.rs      # ZIP operations
├── crypto.rs       # AES-GCM encryption
├── manifest.rs     # Manifest structures
└── policy.rs       # ABAC policy system

crates/
└── mcp-server/     # MCP server implementation
```

### OpenTDFKit
```
OpenTDFKit/
├── NanoTDF.swift           # NanoTDF implementation
├── KASService.swift        # KAS integration
├── KeyStore.swift          # Key management
├── CryptoHelper.swift      # Crypto primitives
└── TDF/
    ├── TDFManifest.swift
    ├── TDFArchive.swift
    └── StandardTDFBuilder.swift
```

### Go SDK
```
sdk/
├── sdk.go              # Main SDK interface
├── tdf.go              # TDF reader/writer
├── nanotdf.go          # NanoTDF implementation
├── kas_client.go       # KAS integration
├── manifest.go         # Manifest structures
└── internal/archive/   # Archive handling
```

---

## Conclusion

**opentdf-rs** has a solid foundation with excellent ABAC policy support and unique MCP integration. However, it lacks critical features for production deployment:

1. **NanoTDF support** - Essential for IoT/mobile use cases
2. **KAS integration** - Required for real-world key management
3. **Streaming** - Important for large file handling

The implementation is well-architected and follows Rust best practices. Adding the missing features would make it a production-ready, differentiated OpenTDF SDK with the best-in-class ABAC system and unique AI integration capabilities.

**Recommended Next Steps**:
1. Prioritize KAS rewrap protocol implementation
2. Add NanoTDF support for spec compliance
3. Implement streaming for production scalability
4. Enhance testing and documentation
5. Create CLI tool for ease of adoption