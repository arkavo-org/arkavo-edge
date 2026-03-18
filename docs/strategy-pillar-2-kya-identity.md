# Pillar 2: Know Your Agent (KYA) - Identity-First Security

## Executive Summary

Implement the **DIF TAAWG MCP-i (Identity)** specification to create a cryptographically verifiable identity layer for all Arkavo-Edge agents. The breakthrough feature: **OpenTDF-to-VC Binding** that links Zero-Trust Data to Zero-Trust Identity.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    DIF TAAWG IDENTITY LAYER                      │
│              (Decentralized Identity Foundation)                 │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─────────────────┐      ┌─────────────────┐                   │
│  │ Agent DID       │      │ Human Admin DID │                   │
│  │ did:arkavo:     │      │ did:web:        │                   │
│  │   agent-123     │      │   enterprise    │                   │
│  │                 │      │   .org:admin-1  │                   │
│  │ ┌───────────┐   │      │ ┌───────────┐   │                   │
│  │ │ Device    │   │      │ │ Enterprise│   │                   │
│  │ │ Binding   │   │      │ │ Wallet    │   │                   │
│  │ │ (TPM/     │   │      │ │ (YubiKey/ │   │                   │
│  │ │  Secure   │   │      │ │  HSM)     │   │                   │
│  │ │  Enclave) │   │      │ └───────────┘   │                   │
│  │ └───────────┘   │      └────────┬────────┘                   │
│  └────────┬────────┘               │                            │
│           │                        │                            │
│           └────────┬───────────────┘                            │
│                    │                                             │
│                    ▼                                             │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │            W3C VERIFIABLE CREDENTIAL (VC) BINDING            ││
│  │                                                              ││
│  │  ┌──────────────────────────────────────────────────────┐   ││
│  │  │ VC: AgentCapability                                    │   ││
│  │  │ {                                                      │   ││
│  │  │   "@context": ["https://www.w3.org/2018/credentials/v1"│   ││
│  │  │   "type": ["VerifiableCredential", "AgentCapability"], │   ││
│  │  │   "issuer": "did:web:enterprise.org",                  │   ││
│  │  │   "credentialSubject": {                               │   ││
│  │  │     "id": "did:arkavo:agent-123",                      │   ││
│  │  │     "role": "data-analyst",                            │   ││
│  │  │     "clearance": "confidential",                       │   ││
│  │  │     "allowedTools": ["query", "analyze"],              │   ││
│  │  │     "expirationDate": "2026-06-01"                     │   ││
│  │  │   },                                                   │   ││
│  │  │   "proof": { ... cryptographic signature ... }         │   ││
│  │  │ }                                                      │   ││
│  │  └──────────────────────────────────────────────────────┘   ││
│  │                                                              ││
│  └─────────────────────────────────────────────────────────────┘│
│                    │                                             │
│                    ▼                                             │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │           OPENTDF KEY ACCESS SERVER (KAS)                    ││
│  │                                                              ││
│  │  Access Control Logic:                                       ││
│  │  ┌─────────────────────────────────────────────────────────┐ ││
│  │  │ if agent_presents_valid_vc(vc, agent_did) AND          │ ││
│  │  │    vc.attributes match policy.attributes AND           │ ││
│  │  │    vc_not_expired(vc) AND                              │ ││
│  │  │    vc_issuer_trusted(vc.issuer)                        │ ││
│  │  │ then                                                   │ ││
│  │  │    issue_decryption_key(policy, agent_did)             │ ││
│  │  │ else                                                   │ ││
│  │  │    deny_access()                                       │ ││
│  │  └─────────────────────────────────────────────────────────┘ ││
│  │                                                              ││
│  └─────────────────────────────────────────────────────────────┘│
│                    │                                             │
└────────────────────┼─────────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────────┐
│                    CRYPTOHITL CHALLENGE                          │
│              (Cryptographic Human-in-the-Loop)                   │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  High-Risk Action Request:                                       │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │ Agent: "Request to execute shell command: rm -rf /data"    │ │
│  │ Risk Level: CRITICAL                                       │ │
│  └────────────────────┬───────────────────────────────────────┘ │
│                       │                                          │
│                       ▼                                          │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │ 1. Generate Challenge                                      │ │
│  │    challenge = hash(action + timestamp + nonce)            │ │
│  │                                                            │ │
│  │ 2. Send to Admin Wallet                                    │ │
│  │    "Sign this challenge to approve action"                 │ │
│  │                                                            │ │
│  │ 3. Admin Signs with DID                                    │ │
│  │    signature = sign(admin_did_key, challenge)              │ │
│  │                                                            │ │
│  │ 4. Verify Signature                                        │ │
│  │    assert(verify(admin_did, signature, challenge))         │ │
│  │                                                            │ │
│  │ 5. Execute Action with Audit Trail                         │ │
│  │    log(approval_did, signature, action_hash)               │ │
│  └────────────────────────────────────────────────────────────┘ │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## DIF TAAWG MCP-i Implementation

### DID Methods

```rust
// crates/arkavo-identity/src/did.rs

/// DIF TAAWG compliant DID implementation
pub mod did {
    use did_doc::Document;
    
    /// Arkavo DID Method
    /// Format: did:arkavo:<agent-id>
    pub struct ArkavoDid;
    
    impl DidMethod for ArkavoDid {
        const METHOD_NAME: &'static str = "arkavo";
        
        fn generate_did(entity: &Entity) -> String {
            // Deterministic DID generation from device binding
            let device_binding = entity.device_binding();
            let hash = blake3::hash(&device_binding);
            format!("did:arkavo:{}", hash.to_hex())
        }
        
        fn resolve(did: &str) -> Result<Document, DidError> {
            // Resolve DID to DID Document
            // 1. Check local cache
            // 2. Query Iroh mesh for agent document
            // 3. Verify device binding
        }
    }
    
    /// Supported DID methods
    pub enum SupportedDid {
        Arkavo(ArkavoDid),
        Web(did_web::DidWeb),      // For enterprise admins
        Key(did_key::DidKey),      // For ephemeral agents
        Jwk(did_jwk::DidJwk),      // For external identities
    }
}

/// Agent Identity Bundle
pub struct AgentIdentity {
    /// Primary DID
    pub did: String,
    
    /// DID Document
    pub document: Document,
    
    /// Device binding (TPM quote / Secure Enclave attestation)
    pub device_binding: DeviceBinding,
    
    /// Current Verifiable Credentials
    pub credentials: Vec<VerifiableCredential>,
    
    /// Capability delegations
    pub delegations: Vec<CapabilityDelegation>,
}
```

### Verifiable Credential Schema

```rust
// crates/arkavo-identity/src/vc.rs

/// W3C Verifiable Credential for Agent Capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilityCredential {
    #[serde(rename = "@context")]
    pub context: Vec<String>,
    
    pub id: String,
    
    #[serde(rename = "type")]
    pub type_: Vec<String>,
    
    pub issuer: Issuer,
    
    #[serde(rename = "issuanceDate")]
    pub issuance_date: DateTime<Utc>,
    
    #[serde(rename = "expirationDate")]
    pub expiration_date: Option<DateTime<Utc>>,
    
    #[serde(rename = "credentialSubject")]
    pub subject: AgentCapabilitySubject,
    
    pub proof: Proof,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilitySubject {
    /// Agent DID
    pub id: String,
    
    /// Role assignment
    pub role: String,
    
    /// Security clearance level
    pub clearance: ClearanceLevel,
    
    /// Allowed tools
    #[serde(rename = "allowedTools")]
    pub allowed_tools: Vec<String>,
    
    /// Allowed data classifications
    #[serde(rename = "allowedDataClasses")]
    pub allowed_data_classes: Vec<String>,
    
    /// Budget limits
    pub budget: BudgetConstraints,
    
    /// Geographic constraints
    #[serde(rename = "allowedRegions")]
    pub allowed_regions: Vec<String>,
}

impl AgentCapabilityCredential {
    /// Verify credential validity
    pub fn verify(&self) -> Result<VerificationResult, VcError> {
        // 1. Verify signature
        let issuer_did = &self.issuer.id;
        let proof_valid = crypto::verify_ed25519(
            &self.proof.proof_value,
            &self.canonicalize(),
            &resolve_key(issuer_did)?,
        )?;
        
        if !proof_valid {
            return Err(VcError::InvalidSignature);
        }
        
        // 2. Check expiration
        if let Some(exp) = self.expiration_date {
            if Utc::now() > exp {
                return Err(VcError::Expired);
            }
        }
        
        // 3. Verify issuer trust
        if !self.is_issuer_trusted() {
            return Err(VcError::UntrustedIssuer);
        }
        
        Ok(VerificationResult::Valid)
    }
}
```

## OpenTDF-to-VC Binding

### Key Access Server (KAS) Enhancement

```rust
// crates/arkavo-tdf/src/vc_kas.rs

/// VC-Aware Key Access Server
/// Only issues decryption keys if agent presents valid VC
pub struct VcAwareKas {
    /// Standard OpenTDF KAS
    inner: OpenTdfKas,
    
    /// VC verification service
    vc_verifier: VcVerifier,
    
    /// Policy engine
    policy_engine: PolicyEngine,
}

impl VcAwareKas {
    /// Access request from agent
    pub async fn request_access(
        &self,
        request: AccessRequest,
    ) -> Result<AccessResponse, KasError> {
        // 1. Authenticate agent via DID
        let agent_did = self.authenticate_agent(&request.auth_token)?;
        
        // 2. Extract and verify VC
        let vc = request.verifiable_credential
            .ok_or(KasError::MissingCredential)?;
        
        let vc_result = self.vc_verifier.verify(&vc).await?;
        if !vc_result.valid {
            return Err(KasError::InvalidCredential(vc_result.reason));
        }
        
        // 3. Verify VC subject matches agent DID
        if vc.subject.id != agent_did {
            return Err(KasError::CredentialSubjectMismatch);
        }
        
        // 4. Evaluate TDF policy against VC attributes
        let policy = self.fetch_policy(&request.policy_id).await?;
        
        let access_decision = self.policy_engine.evaluate(
            &policy,
            &vc.subject,  // Use VC attributes for ABAC
        )?;
        
        if !access_decision.permitted {
            return Err(KasError::PolicyDenied(access_decision.reason));
        }
        
        // 5. Issue wrapped key (standard OpenTDF)
        let wrapped_key = self.inner.issue_key(&request).await?;
        
        // 6. Log access with DID audit trail
        self.audit_log.record(AccessEvent {
            agent_did: agent_did.clone(),
            vc_id: vc.id.clone(),
            policy_id: request.policy_id,
            timestamp: Utc::now(),
            success: true,
        });
        
        Ok(AccessResponse {
            wrapped_key,
            session_id: generate_session_id(),
        })
    }
}

/// VC-to-ABAC Attribute Mapping
impl From<AgentCapabilitySubject> for AbacAttributes {
    fn from(subject: AgentCapabilitySubject) -> Self {
        AbacAttributes {
            role: subject.role,
            clearance: subject.clearance.to_string(),
            tools: subject.allowed_tools,
            data_classes: subject.allowed_data_classes,
            regions: subject.allowed_regions,
        }
    }
}
```

### Zero-Trust Data Flow

```
Data Owner                         TDF Encryption
     │                                     │
     │ 1. Define Policy                    │
     │    - attributes: [clearance:secret] │
     │    - VC requirement: role=analyst   │
     ▼                                     ▼
┌─────────────────────────────────────────────────────────────┐
│                     TDF ENCRYPTED DATA                      │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ Policy:                                               │  │
│  │   - ABAC: clearance:secret                            │  │
│  │   - VC: role=analyst                                  │  │
│  │                                                       │  │
│  │ Payload: AES-256-GCM encrypted                        │  │
│  └───────────────────────────────────────────────────────┘  │
└────────────────────┬────────────────────────────────────────┘
                     │ Store in Iroh
                     ▼
┌─────────────────────────────────────────────────────────────┐
│                     AGENT REQUESTS ACCESS                   │
│                                                              │
│  Agent: did:arkavo:agent-123                                 │
│  VC: {                                                       │
│    issuer: did:web:enterprise.org                            │
│    subject: {                                                │
│      id: did:arkavo:agent-123                                │
│      role: analyst ✓                                         │
│      clearance: secret ✓                                     │
│    }                                                         │
│  }                                                           │
│                                                              │
└────────────────────┬────────────────────────────────────────┘
                     │ Request to KAS
                     ▼
┌─────────────────────────────────────────────────────────────┐
│                     KAS VERIFICATION                        │
│                                                              │
│  Check 1: VC signature valid?              ✓                │
│  Check 2: VC not expired?                  ✓                │
│  Check 3: Issuer trusted?                  ✓                │
│  Check 4: Subject matches agent DID?       ✓                │
│  Check 5: role=analyst matches policy?     ✓                │
│  Check 6: clearance=secret matches policy? ✓                │
│                                                              │
│  Result: ISSUE DECRYPTION KEY                               │
└─────────────────────────────────────────────────────────────┘
```

## CryptoHITL Implementation

### Challenge-Response Flow

```rust
// crates/arkavo-cryptohitl/src/lib.rs

/// Cryptographic Human-in-the-Loop approval system
pub struct CryptoHitl {
    /// Pending challenges
    pending: DashMap<ChallengeId, PendingChallenge>,
    
    /// Approved challenges (for replay protection)
    approved: Arc<RevocationList>,
}

#[derive(Debug, Clone)]
pub struct ActionChallenge {
    /// Unique challenge ID
    pub id: ChallengeId,
    
    /// Action description
    pub action: ActionDescriptor,
    
    /// Risk level
    pub risk_level: RiskLevel,
    
    /// Challenge hash (to be signed)
    pub challenge_hash: [u8; 32],
    
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    
    /// Expiration
    pub expires_at: DateTime<Utc>,
}

impl CryptoHitl {
    /// Initiate HITL approval for high-risk action
    pub async fn request_approval(
        &self,
        action: ActionDescriptor,
        agent: &AgentIdentity,
    ) -> Result<ChallengeId, HitlError> {
        // 1. Validate action requires HITL
        if !action.requires_hitl() {
            return Err(HitlError::NotRequired);
        }
        
        // 2. Generate challenge
        let challenge = ActionChallenge {
            id: ChallengeId::generate(),
            action: action.clone(),
            risk_level: action.risk_level(),
            challenge_hash: self.compute_challenge_hash(&action),
            timestamp: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(5),
        };
        
        // 3. Store pending challenge
        self.pending.insert(
            challenge.id.clone(),
            PendingChallenge {
                challenge: challenge.clone(),
                agent_did: agent.did.clone(),
            },
        );
        
        // 4. Notify administrator (via their DID)
        self.notify_administrators(&challenge).await?;
        
        Ok(challenge.id)
    }
    
    /// Administrator submits approval signature
    pub async fn submit_approval(
        &self,
        challenge_id: ChallengeId,
        approval: AdminApproval,
    ) -> Result<ApprovalReceipt, HitlError> {
        // 1. Retrieve pending challenge
        let pending = self.pending
            .get(&challenge_id)
            .ok_or(HitlError::ChallengeNotFound)?;
        
        // 2. Verify challenge not expired
        if Utc::now() > pending.challenge.expires_at {
            return Err(HitlError::ChallengeExpired);
        }
        
        // 3. Verify admin signature on challenge hash
        let admin_did = &approval.admin_did;
        let signature_valid = crypto::verify_ed25519(
            &approval.signature,
            &pending.challenge.challenge_hash,
            &resolve_key(admin_did)?,
        )?;
        
        if !signature_valid {
            return Err(HitlError::InvalidSignature);
        }
        
        // 4. Verify admin has authority to approve this risk level
        if !self.has_authority(admin_did, pending.challenge.risk_level) {
            return Err(HitlError::InsufficientAuthority);
        }
        
        // 5. Generate approval receipt (non-repudiable proof)
        let receipt = ApprovalReceipt {
            challenge_id: challenge_id.clone(),
            action_hash: pending.challenge.challenge_hash,
            approved_by: admin_did.clone(),
            approved_at: Utc::now(),
            admin_signature: approval.signature,
            kas_wrapped_key: self.generate_wrapped_key(&pending)?,
        };
        
        // 6. Store in approved list (for audit/replay protection)
        self.approved.mark_used(&challenge_id.to_string())?;
        
        // 7. Remove from pending
        self.pending.remove(&challenge_id);
        
        // 8. Audit log
        self.audit.record(HitlApprovalEvent {
            challenge: pending.challenge.clone(),
            approved_by: admin_did.clone(),
            receipt_hash: hash(&receipt),
        });
        
        Ok(receipt)
    }
    
    /// Compute challenge hash for signing
    fn compute_challenge_hash(&self, action: &ActionDescriptor) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(action.canonical_bytes().as_slice());
        hasher.update(&Utc::now().timestamp().to_le_bytes());
        hasher.update(&generate_nonce());
        *hasher.finalize().as_bytes()
    }
}

/// Administrator approval structure
pub struct AdminApproval {
    /// Admin DID
    pub admin_did: String,
    
    /// Signature of challenge hash
    pub signature: Vec<u8>,
    
    /// Optional: Biometric attestation
    pub biometric_attestation: Option<BiometricProof>,
}
```

### Enterprise Wallet Integration

```rust
// Integration with enterprise HSM/wallets

pub enum EnterpriseWallet {
    /// YubiKey hardware token
    YubiKey(YubiKeyProvider),
    
    /// Cloud HSM (AWS KMS, Azure Key Vault, etc.)
    CloudHsm(CloudHsmProvider),
    
    /// Enterprise DID provider
    DidWeb(did_web::DidWeb),
}

impl EnterpriseWallet {
    /// Sign challenge with enterprise key
    pub async fn sign_challenge(
        &self,
        challenge_hash: &[u8; 32],
    ) -> Result<Vec<u8>, WalletError> {
        match self {
            EnterpriseWallet::YubiKey(yk) => {
                yk.sign_ed25519(challenge_hash).await
            }
            EnterpriseWallet::CloudHsm(hsm) => {
                hsm.sign(challenge_hash).await
            }
            EnterpriseWallet::DidWeb(did) => {
                did.sign(challenge_hash).await
            }
        }
    }
}
```

## Implementation Roadmap

### Phase 1: DID Infrastructure (Weeks 1-4)

```rust
// New crate: arkavo-identity

pub mod did;
pub mod vc;
pub mod device_binding;

// Implement:
// - did:arkavo method
// - did:web resolution
// - Device binding (TPM/Secure Enclave)
```

### Phase 2: VC Integration (Weeks 5-8)

```rust
// Extend arkavo-tdf

pub mod vc_kas;

// Implement:
// - VC verification in KAS
// - VC-to-ABAC mapping
// - Credential revocation
```

### Phase 3: CryptoHITL (Weeks 9-12)

```rust
// New crate: arkavo-cryptohitl

pub mod challenge;
pub mod approval;
pub mod wallet_integration;

// Implement:
// - Challenge generation
// - Enterprise wallet integration
// - Audit trail
```

### Phase 4: Ecosystem Integration (Weeks 13-16)

- DIF TAAWG test suite compliance
- Interop with other DID methods
- Enterprise SSO integration (SAML/OIDC → DID)

## Competitive Differentiation

| Feature | Centralized Cloud | Arkavo-Edge |
|---------|------------------|-------------|
| **Agent Identity** | Proprietary tokens | DIF TAAWG-compliant DIDs |
| **Data Access** | IAM policies | OpenTDF + VC binding |
| **HITL** | Web UI click | Cryptographic signature |
| **Audit Trail** | Centralized logs | TDF-encrypted, DID-signed |
| **Interoperability** | Vendor lock-in | Standards-based |
