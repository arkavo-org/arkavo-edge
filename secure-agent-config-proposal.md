# Feature Enhancement: Secure Agent Configuration Distribution with OpenTDF

## Summary

Implement a secure configuration distribution system where a central orchestrator packages and distributes encrypted configuration bundles to multiple agents using OpenTDF (Open Trusted Data Format) for encryption, policy enforcement, and access control. Each bundle contains configuration settings, role definitions, entitlements (permissions), and secrets (credentials, API keys) with fine-grained access control enforced through attribute-based policies.

---

## Problem Statement

### Current Challenges

The Arkavo Edge orchestrator-agent architecture currently lacks a secure, scalable mechanism for distributing sensitive configuration data to agents. This creates several critical problems:

1. **Security Risks**: Configuration data containing API keys, credentials, and sensitive settings are transmitted without encryption or access control, creating potential exposure points for credential theft and unauthorized access.

2. **Access Control Gaps**: There is no fine-grained mechanism to control which agents can access specific configuration elements, leading to over-privileged agents that have access to more data than necessary for their roles.

3. **Audit Trail Deficiency**: Without a structured configuration distribution system, it's difficult to track which agents received what configurations, when they accessed them, and whether they had proper authorization.

4. **Configuration Drift**: Manual or ad-hoc configuration distribution leads to inconsistencies across agents, making it difficult to maintain a known-good state and troubleshoot issues.

5. **Secrets Management**: API keys, tokens, and credentials are currently managed without proper encryption at rest or in transit, violating security best practices and compliance requirements.

6. **Policy Enforcement**: There's no mechanism to enforce data handling policies (e.g., "this API key can only be used by agents with 'production' clearance") or to revoke access dynamically.

### Business Impact

- **Compliance Risk**: Inability to demonstrate proper access controls and audit trails for sensitive data
- **Security Incidents**: Increased attack surface due to unencrypted credentials and over-privileged agents
- **Operational Overhead**: Manual configuration management doesn't scale as agent count grows
- **Trust Boundaries**: Difficulty establishing and maintaining trust relationships between orchestrator and agents

---

## Proposed Solution

### Architecture Overview

Implement a secure configuration distribution system with three core components:

1. **Configuration Bundle Packager** (Orchestrator-side)
2. **OpenTDF Encryption & Policy Engine** (Using opentdf-rs and arkavo-rs KAS)
3. **Secure Configuration Receiver** (Agent-side)

### System Flow

```
┌─────────────────────────────────────────────────────────────────┐
│                        ORCHESTRATOR                              │
│                                                                  │
│  ┌──────────────────┐      ┌─────────────────────────────────┐ │
│  │ Configuration    │      │ OpenTDF Policy Engine           │ │
│  │ Bundle Builder   │─────▶│ - Attribute definitions         │ │
│  │                  │      │ - Access policies               │ │
│  │ - Settings       │      │ - Encryption keys (KAS)         │ │
│  │ - Roles          │      └─────────────────────────────────┘ │
│  │ - Entitlements   │                    │                     │
│  │ - Secrets        │                    ▼                     │
│  └──────────────────┘      ┌─────────────────────────────────┐ │
│                            │ TDF Encrypted Bundle            │ │
│                            │ - Encrypted payload             │ │
│                            │ - Policy manifest               │ │
│                            │ - Access attributes             │ │
│                            └─────────────────────────────────┘ │
└─────────────────────────────────────────┬───────────────────────┘
                                          │
                                          │ Secure Distribution
                                          │ (Public/Private Key + JWT)
                                          ▼
┌─────────────────────────────────────────────────────────────────┐
│                           AGENT                                  │
│                                                                  │
│  ┌──────────────────┐      ┌─────────────────────────────────┐ │
│  │ Agent Identity   │      │ OpenTDF Client                  │ │
│  │ & Credentials    │─────▶│ - Authenticate to KAS           │ │
│  │                  │      │ - Request decryption key        │ │
│  │ - Agent ID       │      │ - Verify attributes             │ │
│  │ - JWT Token      │      └─────────────────────────────────┘ │
│  │ - Attributes     │                    │                     │
│  └──────────────────┘                    ▼                     │
│                            ┌─────────────────────────────────┐ │
│                            │ Decrypted Configuration         │ │
│                            │ - Settings (if authorized)      │ │
│                            │ - Secrets (if authorized)       │ │
│                            │ - Entitlements (if authorized)  │ │
│                            └─────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

### Key Components

#### 1. Configuration Bundle Structure

```rust
pub struct ConfigurationBundle {
    /// Unique bundle identifier
    pub bundle_id: Uuid,
    
    /// Target agent ID or agent role pattern
    pub target: BundleTarget,
    
    /// Configuration settings (non-sensitive)
    pub settings: HashMap<String, serde_json::Value>,
    
    /// Agent role and purpose definition
    pub role: AgentRole,
    
    /// Entitlements (permissions for MCP tools, resources)
    pub entitlements: Vec<Entitlement>,
    
    /// Secrets (API keys, credentials, tokens)
    pub secrets: HashMap<String, Secret>,
    
    /// OpenTDF policy attributes required for access
    pub required_attributes: Vec<String>,
    
    /// Bundle version and metadata
    pub metadata: BundleMetadata,
}

pub enum BundleTarget {
    /// Specific agent by ID
    Agent(String),
    
    /// All agents with specific role
    Role(String),
    
    /// All agents matching attribute pattern
    AttributePattern(String),
}

pub struct AgentRole {
    pub name: String,
    pub purpose: String,
    pub capabilities: Vec<String>,
}

pub struct Entitlement {
    pub resource_type: ResourceType,
    pub resource_id: String,
    pub permissions: Vec<Permission>,
}

pub enum ResourceType {
    McpTool,
    DataProvider,
    FileSystem,
    Network,
}

pub struct Secret {
    pub key: String,
    pub value: String,  // Will be encrypted
    pub secret_type: SecretType,
    pub rotation_policy: Option<RotationPolicy>,
}

pub enum SecretType {
    ApiKey,
    Token,
    Certificate,
    Password,
}
```

#### 2. OpenTDF Integration

**Encryption Process:**

```rust
use opentdf_rs::{TdfClient, Policy, Attribute};

pub struct ConfigBundleEncryptor {
    tdf_client: TdfClient,
    kas_url: String,
}

impl ConfigBundleEncryptor {
    pub async fn encrypt_bundle(
        &self,
        bundle: &ConfigurationBundle,
    ) -> Result<EncryptedBundle> {
        // Define policy attributes
        let policy = Policy::builder()
            .add_attribute(Attribute::new(
                "agent.role",
                &bundle.role.name,
            ))
            .add_attribute(Attribute::new(
                "agent.environment",
                &bundle.metadata.environment,
            ))
            .add_attribute(Attribute::new(
                "data.classification",
                "confidential",
            ))
            .build()?;
        
        // Serialize bundle
        let plaintext = serde_json::to_vec(&bundle)?;
        
        // Encrypt with TDF
        let encrypted = self.tdf_client
            .encrypt(&plaintext, policy)
            .await?;
        
        Ok(EncryptedBundle {
            bundle_id: bundle.bundle_id,
            target: bundle.target.clone(),
            encrypted_data: encrypted,
            policy_manifest: policy.to_manifest(),
            created_at: Utc::now(),
        })
    }
}
```

**Decryption Process (Agent-side):**

```rust
pub struct ConfigBundleDecryptor {
    tdf_client: TdfClient,
    agent_identity: AgentIdentity,
}

impl ConfigBundleDecryptor {
    pub async fn decrypt_bundle(
        &self,
        encrypted: &EncryptedBundle,
    ) -> Result<ConfigurationBundle> {
        // Authenticate to KAS with agent JWT
        let auth_token = self.agent_identity.get_jwt_token()?;
        
        // Request decryption (KAS validates attributes)
        let plaintext = self.tdf_client
            .decrypt(&encrypted.encrypted_data, &auth_token)
            .await?;
        
        // Deserialize bundle
        let bundle: ConfigurationBundle = 
            serde_json::from_slice(&plaintext)?;
        
        Ok(bundle)
    }
}
```

#### 3. Agent Authentication & Authorization

**Public/Private Key Authentication:**

Each agent is provisioned with a unique key pair during registration:
- **Private Key**: Stored securely on the agent, used to sign requests
- **Public Key**: Registered with the orchestrator, used to verify agent identity

```rust
pub struct AgentIdentity {
    pub agent_id: String,
    pub attributes: HashMap<String, String>,
    pub jwt_token: String,
    pub private_key: PrivateKey,  // Agent's private key for signing
    pub public_key: PublicKey,    // Agent's public key (registered with orchestrator)
}

impl AgentIdentity {
    /// Generate JWT token with agent attributes
    pub fn generate_jwt(&self, kas_url: &str) -> Result<String> {
        let claims = Claims {
            sub: self.agent_id.clone(),
            iss: "arkavo-orchestrator",
            aud: kas_url.to_string(),
            exp: (Utc::now() + Duration::hours(1)).timestamp(),
            attributes: self.attributes.clone(),
        };
        
        // Sign with orchestrator's private key
        encode(&Header::default(), &claims, &ENCODING_KEY)
    }
    
    /// Sign a request with agent's private key
    pub fn sign_request(&self, request_data: &[u8]) -> Result<Signature> {
        use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
        
        let key_pair = EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_ASN1_SIGNING,
            self.private_key.as_bytes(),
        )?;
        
        let rng = ring::rand::SystemRandom::new();
        let signature = key_pair.sign(&rng, request_data)?;
        
        Ok(Signature::from_bytes(signature.as_ref()))
    }
    
    /// Verify agent has required attributes for bundle
    pub fn has_required_attributes(
        &self,
        required: &[String],
    ) -> bool {
        required.iter().all(|attr| {
            self.attributes.contains_key(attr)
        })
    }
}

/// Orchestrator verifies agent requests using public key
pub struct AgentVerifier {
    registered_agents: HashMap<String, PublicKey>,
}

impl AgentVerifier {
    /// Verify request signature using agent's public key
    pub fn verify_request(
        &self,
        agent_id: &str,
        request_data: &[u8],
        signature: &Signature,
    ) -> Result<bool> {
        use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1};
        
        let public_key = self.registered_agents
            .get(agent_id)
            .ok_or_else(|| Error::UnknownAgent)?;
        
        let public_key = UnparsedPublicKey::new(
            &ECDSA_P256_SHA256_ASN1,
            public_key.as_bytes(),
        );
        
        public_key.verify(request_data, signature.as_bytes())
            .map(|_| true)
            .or(Ok(false))
    }
}
```

#### 4. Configuration Distribution API

**Orchestrator Endpoints:**

```rust
#[rpc(server)]
pub trait ConfigDistribution {
    /// Request configuration bundle for agent (with signature verification)
    #[method(name = "config.request")]
    async fn request_config(
        &self,
        agent_id: String,
        agent_jwt: String,
        request_signature: String,  // Request signed with agent's private key
    ) -> RpcResult<EncryptedBundle>;
    
    /// Acknowledge configuration receipt (with signature verification)
    #[method(name = "config.acknowledge")]
    async fn acknowledge_config(
        &self,
        bundle_id: Uuid,
        agent_id: String,
        signature: String,  // Acknowledgment signed with agent's private key
    ) -> RpcResult<()>;
    
    /// Report configuration application status (with signature verification)
    #[method(name = "config.status")]
    async fn report_status(
        &self,
        bundle_id: Uuid,
        status: ConfigStatus,
        signature: String,  // Status report signed with agent's private key
    ) -> RpcResult<()>;
}
```

**Agent Client:**

```rust
pub struct ConfigClient {
    orchestrator_url: String,
    agent_identity: AgentIdentity,
    decryptor: ConfigBundleDecryptor,
}

impl ConfigClient {
    pub async fn fetch_configuration(&self) -> Result<ConfigurationBundle> {
        // Create request data
        let request_data = format!(
            "{}:{}:{}",
            self.agent_identity.agent_id,
            self.agent_identity.jwt_token,
            Utc::now().timestamp()
        );
        
        // Sign request with agent's private key
        let signature = self.agent_identity.sign_request(request_data.as_bytes())?;
        
        // Request encrypted bundle from orchestrator (with signature)
        let encrypted = self.request_bundle(&signature).await?;
        
        // Decrypt using OpenTDF
        let bundle = self.decryptor
            .decrypt_bundle(&encrypted)
            .await?;
        
        // Sign acknowledgment
        let ack_data = format!("{}:{}", bundle.bundle_id, Utc::now().timestamp());
        let ack_signature = self.agent_identity.sign_request(ack_data.as_bytes())?;
        
        // Acknowledge receipt (with signature)
        self.acknowledge_bundle(&bundle.bundle_id, &ack_signature).await?;
        
        // Apply configuration
        self.apply_configuration(&bundle).await?;
        
        Ok(bundle)
    }
}
```

### Security Features

1. **End-to-End Encryption**: All configuration data encrypted with OpenTDF before transmission
2. **Attribute-Based Access Control (ABAC)**: Fine-grained policies based on agent attributes
3. **Key Management**: Centralized KAS (Key Access Service) using arkavo-rs implementation
4. **Public/Private Key Authentication**: Agent identity verification using asymmetric cryptography
5. **JWT Authentication**: Agent identity verification with signed tokens
6. **Audit Logging**: Complete trail of configuration access and modifications
7. **Policy Enforcement**: Dynamic access control with revocation capabilities
8. **Secrets Rotation**: Automated rotation policies for sensitive credentials

---

## Technical Requirements

### Dependencies

```toml
[dependencies]
# OpenTDF Rust implementation
opentdf-rs = { git = "https://github.com/arkavo-org/opentdf-rs" }

# Arkavo KAS implementation
arkavo-kas = { git = "https://github.com/arkavo-org/arkavo-rs" }

# Existing dependencies
uuid = { version = "1.0", features = ["v4", "serde"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1.0", features = ["full"] }
jsonrpsee = { version = "0.20", features = ["server", "client"] }
chrono = { version = "0.4", features = ["serde"] }
```

### New Crates

Create the following crates following Arkavo's one-crate-per-capability principle:

1. **arkavo-config-bundle** (< 400 LOC)
   - Configuration bundle data structures
   - Bundle serialization/deserialization
   - Bundle validation logic

2. **arkavo-config-encryption** (< 400 LOC)
   - OpenTDF integration
   - Encryption/decryption operations
   - Policy management

3. **arkavo-config-distribution** (< 400 LOC)
   - Orchestrator-side distribution logic
   - Agent registration and tracking
   - Bundle routing and delivery

4. **arkavo-config-client** (< 400 LOC)
   - Agent-side configuration client
   - Bundle fetching and decryption
   - Configuration application

### API Endpoints

**Orchestrator (Server-side):**

```rust
// Configuration management
POST   /api/v1/config/bundle/create      // Create new bundle
GET    /api/v1/config/bundle/{id}        // Get bundle details
PUT    /api/v1/config/bundle/{id}        // Update bundle
DELETE /api/v1/config/bundle/{id}        // Delete bundle

// Distribution
POST   /api/v1/config/distribute         // Distribute bundle to agents
GET    /api/v1/config/agent/{id}/current // Get agent's current config

// Policy management
POST   /api/v1/policy/create             // Create access policy
GET    /api/v1/policy/{id}               // Get policy details
PUT    /api/v1/policy/{id}               // Update policy
DELETE /api/v1/policy/{id}               // Delete policy

// Audit
GET    /api/v1/audit/config/access       // Config access logs
GET    /api/v1/audit/config/changes      // Config change history
```

**Agent (Client-side):**

```rust
// Configuration retrieval
GET    /api/v1/config/request            // Request configuration bundle
POST   /api/v1/config/acknowledge        // Acknowledge receipt
POST   /api/v1/config/status             // Report application status

// Identity & authentication
POST   /api/v1/agent/register            // Register agent identity
POST   /api/v1/agent/heartbeat           // Maintain session
```

### Data Structures

**Configuration Bundle Schema:**

```json
{
  "bundle_id": "550e8400-e29b-41d4-a716-446655440000",
  "target": {
    "type": "role",
    "value": "search-agent"
  },
  "settings": {
    "max_concurrent_tasks": 5,
    "timeout_seconds": 30,
    "log_level": "info"
  },
  "role": {
    "name": "search-agent",
    "purpose": "Web search and data retrieval",
    "capabilities": ["web_search", "data_extraction"]
  },
  "entitlements": [
    {
      "resource_type": "mcp_tool",
      "resource_id": "web-search",
      "permissions": ["execute", "read_results"]
    }
  ],
  "secrets": {
    "google_api_key": {
      "key": "GOOGLE_API_KEY",
      "value": "[ENCRYPTED]",
      "secret_type": "api_key",
      "rotation_policy": {
        "interval_days": 90,
        "auto_rotate": true
      }
    }
  },
  "required_attributes": [
    "agent.role=search-agent",
    "agent.environment=production",
    "security.clearance=standard"
  ],
  "metadata": {
    "version": "1.0.0",
    "created_at": "2025-01-15T10:30:00Z",
    "created_by": "orchestrator-admin",
    "environment": "production"
  }
}
```

**OpenTDF Policy Manifest:**

```json
{
  "policy": {
    "uuid": "policy-123",
    "body": {
      "dataAttributes": [
        {
          "attribute": "agent.role",
          "displayName": "Agent Role",
          "pubKey": "[PUBLIC_KEY]",
          "kasUrl": "https://kas.arkavo.local"
        },
        {
          "attribute": "security.clearance",
          "displayName": "Security Clearance",
          "pubKey": "[PUBLIC_KEY]",
          "kasUrl": "https://kas.arkavo.local"
        }
      ],
      "dissem": [
        "agent.role=search-agent",
        "security.clearance=standard"
      ]
    }
  }
}
```

### Integration Points

1. **Existing Agent Registry** (`crates/arkavo-protocol/src/agent_registry.rs`)
   - Extend to store agent attributes for ABAC
   - Add configuration bundle tracking per agent

2. **Task Planner** (`crates/arkavo-protocol/src/task_planner.rs`)
   - Use configuration bundles to determine agent capabilities
   - Validate entitlements before task assignment

3. **Authorization Module** (`crates/arkavo-authorization/`)
   - Integrate OpenTDF authorization checks
   - Extend with configuration-specific policies

4. **Protocol Server** (`crates/arkavo-protocol/src/server.rs`)
   - Add configuration distribution endpoints
   - Implement bundle request/response handlers

---

## User Stories

### Story 1: System Administrator - Creating Secure Agent Configurations

**As a** system administrator managing the Arkavo orchestrator,  
**I want to** create encrypted configuration bundles with fine-grained access policies,  
**So that** I can securely distribute sensitive credentials and settings to agents without risk of unauthorized access.

**Acceptance Criteria:**
- [ ] Can create configuration bundles through CLI or UI
- [ ] Can specify target agents by ID, role, or attribute pattern
- [ ] Can define access policies using OpenTDF attributes
- [ ] Can include secrets (API keys, tokens) that are automatically encrypted
- [ ] Can set entitlements (MCP tool permissions) per bundle
- [ ] Receive confirmation when bundle is created and encrypted
- [ ] Can view bundle metadata without decrypting contents

**Example Workflow:**
```bash
# Create configuration bundle for search agents
arkavo config create \
  --target role:search-agent \
  --role "Web Search Agent" \
  --capability web_search \
  --secret GOOGLE_API_KEY=sk-xxx \
  --entitlement mcp_tool:web-search:execute \
  --attribute agent.role=search-agent \
  --attribute security.clearance=standard \
  --output search-agent-config.tdf

# Output:
# ✓ Configuration bundle created: bundle-550e8400
# ✓ Encrypted with OpenTDF
# ✓ Policy: agent.role=search-agent AND security.clearance=standard
# ✓ Secrets: 1 encrypted
# ✓ Entitlements: 1 defined
# ✓ Ready for distribution
```

### Story 2: Agent - Requesting and Applying Configuration

**As an** Arkavo agent starting up,  
**I want to** automatically request my configuration bundle from the orchestrator and decrypt it using my identity,  
**So that** I can operate with the correct settings, credentials, and permissions without manual configuration.

**Acceptance Criteria:**
- [ ] Agent automatically requests configuration on startup
- [ ] Agent authenticates using JWT token with attributes
- [ ] Agent receives encrypted bundle from orchestrator
- [ ] Agent decrypts bundle using OpenTDF client and KAS
- [ ] Decryption fails if agent lacks required attributes
- [ ] Agent applies configuration (settings, secrets, entitlements)
- [ ] Agent acknowledges successful configuration receipt
- [ ] Agent logs configuration version and timestamp

**Example Workflow:**
```bash
# Agent startup
arkavo agent run --id search-agent-001

# Logs:
# [INFO] Agent starting: search-agent-001
# [INFO] Requesting configuration from orchestrator
# [INFO] Authenticating to KAS with JWT
# [INFO] Received encrypted bundle: bundle-550e8400
# [INFO] Decrypting with OpenTDF...
# [INFO] ✓ Configuration decrypted successfully
# [INFO] Applying configuration:
#        - Role: Web Search Agent
#        - Capabilities: web_search
#        - Secrets: GOOGLE_API_KEY (loaded)
#        - Entitlements: mcp_tool:web-search:execute
# [INFO] Configuration acknowledged
# [INFO] Agent ready
```

### Story 3: Security Auditor - Reviewing Access Logs

**As a** security auditor,  
**I want to** review comprehensive logs of configuration access attempts and policy decisions,  
**So that** I can verify compliance, detect unauthorized access attempts, and investigate security incidents.

**Acceptance Criteria:**
- [ ] Can query configuration access logs by agent, bundle, or time range
- [ ] Logs include: agent ID, bundle ID, timestamp, success/failure, attributes used
- [ ] Can see policy evaluation decisions (allow/deny with reasons)
- [ ] Can track configuration changes over time
- [ ] Can identify agents that attempted unauthorized access
- [ ] Can export logs for compliance reporting
- [ ] Logs are tamper-evident and immutable

**Example Workflow:**
```bash
# Query access logs
arkavo audit config-access \
  --bundle bundle-550e8400 \
  --since 2025-01-01 \
  --format json

# Output:
[
  {
    "timestamp": "2025-01-15T10:35:22Z",
    "agent_id": "search-agent-001",
    "bundle_id": "bundle-550e8400",
    "action": "decrypt",
    "result": "allowed",
    "policy_decision": {
      "required_attributes": [
        "agent.role=search-agent",
        "security.clearance=standard"
      ],
      "agent_attributes": {
        "agent.role": "search-agent",
        "security.clearance": "standard",
        "agent.environment": "production"
      },
      "evaluation": "all_required_attributes_present"
    }
  },
  {
    "timestamp": "2025-01-15T10:36:05Z",
    "agent_id": "rogue-agent-999",
    "bundle_id": "bundle-550e8400",
    "action": "decrypt",
    "result": "denied",
    "policy_decision": {
      "required_attributes": [
        "agent.role=search-agent",
        "security.clearance=standard"
      ],
      "agent_attributes": {
        "agent.role": "unknown",
        "security.clearance": "none"
      },
      "evaluation": "missing_required_attribute: agent.role",
      "reason": "Agent does not have required role attribute"
    }
  }
]
```

### Story 4: DevOps Engineer - Rotating Secrets

**As a** DevOps engineer,  
**I want to** rotate API keys and credentials in configuration bundles without disrupting agent operations,  
**So that** I can maintain security hygiene and respond to potential credential compromises.

**Acceptance Criteria:**
- [ ] Can update secrets in existing configuration bundles
- [ ] New bundle version is created with rotated secrets
- [ ] Agents automatically fetch updated configuration
- [ ] Old secrets remain valid during grace period
- [ ] Can force immediate rotation for compromised credentials
- [ ] Rotation events are logged for audit trail
- [ ] Can configure automatic rotation policies

**Example Workflow:**
```bash
# Rotate API key in bundle
arkavo config rotate-secret \
  --bundle bundle-550e8400 \
  --secret GOOGLE_API_KEY \
  --new-value sk-new-key-xxx \
  --grace-period 1h

# Output:
# ✓ Secret rotation initiated
# ✓ New bundle version: bundle-550e8400-v2
# ✓ Old key valid until: 2025-01-15T11:35:00Z
# ✓ Notifying 5 agents with this configuration
# ✓ Agents will auto-update on next heartbeat
#
# Rotation status:
# - search-agent-001: ✓ Updated (10:35:45)
# - search-agent-002: ✓ Updated (10:36:12)
# - search-agent-003: ⏳ Pending
# - search-agent-004: ✓ Updated (10:35:58)
# - search-agent-005: ⏳ Pending
```

### Story 5: Agent Developer - Testing with Local Configuration

**As an** agent developer,  
**I want to** test my agent with local configuration bundles without connecting to the orchestrator,  
**So that** I can develop and debug in isolation before deploying to production.

**Acceptance Criteria:**
- [ ] Can create local configuration bundles for testing
- [ ] Can run agent with `--config-file` flag pointing to local bundle
- [ ] Local bundles use same structure as orchestrator bundles
- [ ] Can test with mock secrets and entitlements
- [ ] Can validate bundle structure before deployment
- [ ] Development mode bypasses OpenTDF encryption for easier debugging
- [ ] Can export production-ready encrypted bundle from local config

**Example Workflow:**
```bash
# Create local test configuration
cat > test-config.json <<EOF
{
  "bundle_id": "test-bundle-001",
  "target": {"type": "agent", "value": "test-agent"},
  "settings": {
    "log_level": "debug",
    "max_concurrent_tasks": 1
  },
  "role": {
    "name": "test-agent",
    "purpose": "Development testing",
    "capabilities": ["web_search"]
  },
  "secrets": {
    "test_api_key": {
      "key": "TEST_API_KEY",
      "value": "test-key-123",
      "secret_type": "api_key"
    }
  },
  "entitlements": [
    {
      "resource_type": "mcp_tool",
      "resource_id": "web-search",
      "permissions": ["execute"]
    }
  ]
}
EOF

# Run agent with local config
arkavo agent run \
  --id test-agent \
  --config-file test-config.json \
  --dev-mode

# Output:
# [INFO] Development mode enabled
# [INFO] Loading local configuration: test-config.json
# [INFO] ✓ Configuration loaded (unencrypted)
# [INFO] Agent ready for testing
```

---

## Acceptance Criteria

### Core Functionality

- [ ] **Bundle Creation**: Orchestrator can create configuration bundles with settings, roles, entitlements, and secrets
- [ ] **OpenTDF Encryption**: All bundles are encrypted using OpenTDF with attribute-based policies
- [ ] **KAS Integration**: Arkavo-rs KAS implementation handles key management and policy enforcement
- [ ] **Agent Authentication**: Agents authenticate using JWT tokens with embedded attributes
- [ ] **Secure Distribution**: Bundles distributed with public/private key authentication
- [ ] **Decryption Authorization**: Agents can only decrypt bundles if they possess required attributes
- [ ] **Configuration Application**: Agents successfully apply decrypted configuration (settings, secrets, entitlements)
- [ ] **Audit Logging**: All configuration access attempts logged with policy decisions

### Security Requirements

- [ ] **Encryption at Rest**: Configuration bundles stored encrypted in orchestrator database
- [ ] **Encryption in Transit**: All bundles signed with private keys and verified with public keys
- [ ] **Key Rotation**: Support for rotating encryption keys and secrets
- [ ] **Access Revocation**: Ability to revoke agent access to configurations dynamically
- [ ] **Attribute Validation**: KAS validates agent attributes before granting decryption keys
- [ ] **Audit Trail**: Immutable logs of all configuration operations
- [ ] **Secrets Management**: Secrets never logged or exposed in plaintext
- [ ] **Policy Enforcement**: OpenTDF policies correctly enforce access control

### Performance Requirements

- [ ] **Bundle Creation**: < 100ms to create and encrypt bundle
- [ ] **Bundle Distribution**: < 500ms end-to-end from request to decryption
- [ ] **KAS Response Time**: < 50ms for key access decisions
- [ ] **Agent Startup**: Configuration fetch adds < 2s to agent startup time
- [ ] **Scalability**: Support 1000+ agents requesting configurations concurrently

### Integration Requirements

- [ ] **Agent Registry Integration**: Configuration system uses existing agent registry
- [ ] **Task Planner Integration**: Task planner validates entitlements from configuration
- [ ] **Authorization Module Integration**: Leverages arkavo-authorization for policy checks
- [ ] **Protocol Server Integration**: Configuration endpoints added to A2A protocol server
- [ ] **CLI Integration**: Configuration management commands added to arkavo CLI
- [ ] **UI Integration**: Configuration management UI in arkavo web interface

### Testing Requirements

- [ ] **Unit Tests**: All components have >85% code coverage
- [ ] **Integration Tests**: End-to-end tests for bundle creation, distribution, and decryption
- [ ] **Security Tests**: Penetration testing of encryption and access control
- [ ] **Performance Tests**: Load testing with 1000+ concurrent agents
- [ ] **Failure Tests**: Graceful handling of KAS unavailability, network failures, invalid tokens
- [ ] **Regression Tests**: All tests pass in CI/CD pipeline

### Documentation Requirements

- [ ] **Architecture Documentation**: System design and component interactions documented
- [ ] **API Documentation**: All endpoints documented with examples
- [ ] **User Guide**: Step-by-step guide for creating and distributing configurations
- [ ] **Security Guide**: Best practices for secrets management and policy design
- [ ] **Developer Guide**: Instructions for integrating configuration system into agents
- [ ] **Troubleshooting Guide**: Common issues and solutions

---

## Security Considerations

### Threat Model

**Threats Addressed:**

1. **Credential Theft**: Encrypted secrets prevent theft in transit or at rest
2. **Unauthorized Access**: ABAC policies prevent agents from accessing configurations they shouldn't
3. **Man-in-the-Middle**: Public/private key signatures prevent interception and tampering
4. **Privilege Escalation**: Entitlements limit what agents can do even with valid configuration
5. **Insider Threats**: Audit logs provide accountability and detection
6. **Key Compromise**: Key rotation and revocation limit blast radius

**Threats Not Addressed (Out of Scope):**

- Memory scraping on agent systems (requires OS-level protections)
- Physical access to agent hardware (requires hardware security modules)
- Social engineering of administrators (requires organizational policies)

### Encryption Standards

- **Algorithm**: AES-256-GCM for symmetric encryption
- **Key Exchange**: ECDH with P-256 curve
- **Signatures**: ECDSA with P-256 curve for request signing and verification
- **Transport**: HTTPS with certificate pinning for additional security
- **JWT**: RS256 (RSA with SHA-256) for token signing

### Key Management

- **KAS (Key Access Service)**: Centralized key management using arkavo-rs
- **Key Storage**: Keys stored in encrypted key store with access controls
- **Key Rotation**: Automated rotation every 90 days (configurable)
- **Key Backup**: Encrypted backups with split-key recovery
- **Key Destruction**: Secure deletion when keys are rotated

### Access Control

- **Attribute-Based Access Control (ABAC)**: Fine-grained policies based on agent attributes
- **Least Privilege**: Agents only receive configurations they need
- **Dynamic Revocation**: Ability to revoke access without re-encrypting bundles
- **Temporal Policies**: Time-based access restrictions (e.g., "only during business hours")
- **Contextual Policies**: Location, network, or device-based restrictions

### Audit & Compliance

- **Comprehensive Logging**: All configuration operations logged
- **Tamper-Evident Logs**: Logs use cryptographic hashing to prevent modification
- **Retention Policies**: Configurable log retention (default: 1 year)
- **Compliance Support**: Logs structured for SOC 2, ISO 27001, GDPR compliance
- **Alerting**: Real-time alerts for suspicious access patterns

### Secrets Management Best Practices

1. **Never Log Secrets**: Secrets redacted from all logs
2. **Rotate Regularly**: Automated rotation policies enforced
3. **Limit Scope**: Secrets scoped to minimum required permissions
4. **Monitor Usage**: Track secret usage for anomaly detection
5. **Revoke Immediately**: Instant revocation on compromise detection

### Secure Development Practices

- **Code Review**: All security-critical code requires peer review
- **Static Analysis**: Automated security scanning in CI/CD
- **Dependency Scanning**: Regular audits of third-party dependencies
- **Penetration Testing**: Annual third-party security assessments
- **Incident Response**: Documented procedures for security incidents

---

## Implementation Plan

### Phase 1: Core Infrastructure (Week 1-2)

**Deliverables:**
- [ ] Create `arkavo-config-bundle` crate with data structures
- [ ] Create `arkavo-config-encryption` crate with OpenTDF integration
- [ ] Integrate opentdf-rs and arkavo-rs KAS
- [ ] Implement bundle encryption/decryption
- [ ] Unit tests for core components

**Estimated Effort:** 40 hours

### Phase 2: Distribution System (Week 3-4)

**Deliverables:**
- [ ] Create `arkavo-config-distribution` crate
- [ ] Implement orchestrator-side bundle management
- [ ] Add configuration endpoints to protocol server
- [ ] Implement agent authentication with JWT
- [ ] Integration tests for distribution flow

**Estimated Effort:** 40 hours

### Phase 3: Agent Integration (Week 5)

**Deliverables:**
- [ ] Create `arkavo-config-client` crate
- [ ] Implement agent-side configuration fetching
- [ ] Integrate with existing agent startup
- [ ] Add configuration application logic
- [ ] End-to-end tests with real agents

**Estimated Effort:** 20 hours

### Phase 4: Security & Audit (Week 6)

**Deliverables:**
- [ ] Implement comprehensive audit logging
- [ ] Add access control policy enforcement
- [ ] Implement secrets rotation
- [ ] Security testing and hardening
- [ ] Penetration testing

**Estimated Effort:** 30 hours

### Phase 5: CLI & UI (Week 7)

**Deliverables:**
- [ ] Add configuration management commands to CLI
- [ ] Create web UI for bundle management
- [ ] Add audit log viewer
- [ ] User documentation
- [ ] Tutorial videos

**Estimated Effort:** 25 hours

### Phase 6: Testing & Documentation (Week 8)

**Deliverables:**
- [ ] Performance testing and optimization
- [ ] Complete documentation
- [ ] Deployment guides
- [ ] Troubleshooting guides
- [ ] Release preparation

**Estimated Effort:** 20 hours

**Total Estimated Effort:** 175 hours (approximately 8 weeks)

---

## Dependencies

### External Dependencies

- **opentdf-rs**: OpenTDF Rust implementation for encryption and policy enforcement
  - Repository: https://github.com/arkavo-org/opentdf-rs
  - Status: Active development
  - Integration: Direct dependency

- **arkavo-rs**: Arkavo KAS (Key Access Service) implementation
  - Repository: https://github.com/arkavo-org/arkavo-rs
  - Status: Active development
  - Integration: Direct dependency for key management

### Internal Dependencies

- **arkavo-protocol**: Agent registry and protocol server
- **arkavo-authorization**: Existing authorization framework
- **arkavo-cli**: Command-line interface
- **arkavo-agui**: Web UI framework

### System Dependencies

- **Rust**: 1.70+ (for latest async features)
- **OpenSSL**: 3.0+ (or rustls as alternative)
- **PostgreSQL**: 14+ (for audit log storage)

---

## Risks & Mitigations

### Technical Risks

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| OpenTDF integration complexity | High | Medium | Start with simple policies, iterate |
| KAS performance bottleneck | High | Low | Implement caching, load balancing |
| Key rotation disrupts agents | Medium | Medium | Implement grace periods, gradual rollout |
| Encryption overhead impacts performance | Medium | Low | Benchmark early, optimize hot paths |

### Security Risks

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| KAS compromise exposes all secrets | Critical | Very Low | Multi-layer encryption, HSM integration |
| Policy misconfiguration grants excessive access | High | Medium | Policy validation, dry-run mode, auditing |
| JWT token theft | High | Low | Short expiration, token rotation, signed requests |
| Side-channel attacks on decryption | Medium | Very Low | Constant-time operations, secure memory |

### Operational Risks

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| Configuration drift across agents | Medium | Medium | Version tracking, automated validation |
| Audit log storage growth | Low | High | Automated archival, retention policies |
| Administrator error in policy creation | High | Medium | Policy templates, validation, rollback |
| KAS unavailability blocks agent startup | High | Low | Cached configurations, fallback mode |

---

## Success Metrics

### Adoption Metrics

- **Agent Coverage**: >90% of agents using secure configuration within 3 months
- **Bundle Creation**: >100 configuration bundles created in first month
- **Active Policies**: >50 unique access policies defined

### Security Metrics

- **Unauthorized Access Attempts**: <1% of total access attempts
- **Policy Violations**: 0 successful policy bypasses
- **Audit Coverage**: 100% of configuration operations logged
- **Incident Response Time**: <1 hour to revoke compromised credentials

### Performance Metrics

- **Configuration Fetch Time**: <500ms p95
- **KAS Response Time**: <50ms p95
- **Bundle Encryption Time**: <100ms p95
- **System Availability**: >99.9% uptime

### User Satisfaction

- **Administrator Satisfaction**: >4.5/5 rating for ease of use
- **Developer Satisfaction**: >4.0/5 rating for integration experience
- **Documentation Quality**: >4.5/5 rating for completeness

---

## Future Enhancements

### Phase 2 Features (Post-MVP)

1. **Hardware Security Module (HSM) Integration**
   - Store master keys in HSM for enhanced security
   - FIPS 140-2 Level 3 compliance

2. **Multi-Tenancy Support**
   - Isolated configuration namespaces per tenant
   - Tenant-specific KAS instances

3. **Configuration Templates**
   - Pre-defined templates for common agent roles
   - Template marketplace for sharing configurations

4. **Dynamic Policy Updates**
   - Update policies without re-encrypting bundles
   - Real-time policy propagation to KAS

5. **Advanced Audit Analytics**
   - ML-based anomaly detection
   - Predictive security alerts
   - Compliance dashboards

6. **Configuration Versioning & Rollback**
   - Git-like versioning for configurations
   - One-click rollback to previous versions
   - Diff visualization between versions

7. **Secrets Vault Integration**
   - Integration with HashiCorp Vault
   - Integration with AWS Secrets Manager
   - Integration with Azure Key Vault

8. **Zero-Trust Architecture**
   - Continuous authentication
   - Micro-segmentation of agent networks
   - Just-in-time access provisioning

---

## References

### Technical Documentation

- [OpenTDF Specification](https://github.com/opentdf/spec)
- [OpenTDF Rust Implementation](https://github.com/arkavo-org/opentdf-rs)
- [Arkavo KAS Implementation](https://github.com/arkavo-org/arkavo-rs)
- [Arkavo Edge Architecture](docs/ORCHESTRATOR_IMPLEMENTATION.md)

### Security Standards

- [NIST SP 800-57: Key Management](https://csrc.nist.gov/publications/detail/sp/800-57-part-1/rev-5/final)
- [NIST SP 800-63B: Digital Identity Guidelines](https://pages.nist.gov/800-63-3/sp800-63b.html)
- [OWASP API Security Top 10](https://owasp.org/www-project-api-security/)

### Compliance Frameworks

- [SOC 2 Type II](https://www.aicpa.org/interestareas/frc/assuranceadvisoryservices/aicpasoc2report.html)
- [ISO 27001](https://www.iso.org/isoiec-27001-information-security.html)
- [GDPR](https://gdpr.eu/)

---

## Appendix

### Glossary

- **ABAC**: Attribute-Based Access Control - access control based on attributes of subjects, objects, and environment
- **KAS**: Key Access Service - centralized service for managing encryption keys and access policies
- **OpenTDF**: Open Trusted Data Format - open standard for encrypting data with policy enforcement
- **TDF**: Trusted Data Format - encrypted data container with embedded access policies
- **JWT**: JSON Web Token - compact token format for securely transmitting information
- **Public/Private Key Authentication**: Asymmetric cryptography for agent identity verification and request signing
- **ECDH**: Elliptic Curve Diffie-Hellman - key exchange protocol
- **ECDSA**: Elliptic Curve Digital Signature Algorithm - digital signature algorithm

### Example Configuration Bundle (Full)

```json
{
  "bundle_id": "550e8400-e29b-41d4-a716-446655440000",
  "target": {
    "type": "role",
    "value": "search-agent"
  },
  "settings": {
    "max_concurrent_tasks": 5,
    "timeout_seconds": 30,
    "log_level": "info",
    "retry_attempts": 3,
    "retry_backoff_ms": 1000,
    "health_check_interval_seconds": 60
  },
  "role": {
    "name": "search-agent",
    "purpose": "Web search and data retrieval for user queries",
    "capabilities": [
      "web_search",
      "data_extraction",
      "content_summarization"
    ]
  },
  "entitlements": [
    {
      "resource_type": "mcp_tool",
      "resource_id": "web-search",
      "permissions": ["execute", "read_results"]
    },
    {
      "resource_type": "mcp_tool",
      "resource_id": "scrape-webpage",
      "permissions": ["execute", "read_results"]
    },
    {
      "resource_type": "data_provider",
      "resource_id": "google-search",
      "permissions": ["query", "read"]
    },
    {
      "resource_type": "network",
      "resource_id": "https://*.google.com",
      "permissions": ["connect", "read"]
    }
  ],
  "secrets": {
    "google_api_key": {
      "key": "GOOGLE_API_KEY",
      "value": "[ENCRYPTED_BY_OPENTDF]",
      "secret_type": "api_key",
      "rotation_policy": {
        "interval_days": 90,
        "auto_rotate": true,
        "notify_before_days": 7
      },
      "metadata": {
        "created_at": "2025-01-01T00:00:00Z",
        "last_rotated": "2025-01-01T00:00:00Z",
        "next_rotation": "2025-04-01T00:00:00Z"
      }
    },
    "api_rate_limit_token": {
      "key": "RATE_LIMIT_TOKEN",
      "value": "[ENCRYPTED_BY_OPENTDF]",
      "secret_type": "token",
      "rotation_policy": {
        "interval_days": 30,
        "auto_rotate": true
      }
    }
  },
  "required_attributes": [
    "agent.role=search-agent",
    "agent.environment=production",
    "security.clearance=standard",
    "network.zone=dmz"
  ],
  "metadata": {
    "version": "1.2.0",
    "created_at": "2025-01-15T10:30:00Z",
    "created_by": "admin@arkavo.org",
    "updated_at": "2025-01-15T10:30:00Z",
    "updated_by": "admin@arkavo.org",
    "environment": "production",
    "tags": ["search", "web", "production"],
    "description": "Production configuration for web search agents",
    "change_log": [
      {
        "version": "1.2.0",
        "timestamp": "2025-01-15T10:30:00Z",
        "author": "admin@arkavo.org",
        "changes": "Added scrape-webpage entitlement"
      },
      {
        "version": "1.1.0",
        "timestamp": "2025-01-10T14:20:00Z",
        "author": "admin@arkavo.org",
        "changes": "Increased max_concurrent_tasks to 5"
      },
      {
        "version": "1.0.0",
        "timestamp": "2025-01-01T00:00:00Z",
        "author": "admin@arkavo.org",
        "changes": "Initial configuration"
      }
    ]
  }
}
```

---

**Document Version:** 1.0  
**Last Updated:** 2025-01-15  
**Status:** Proposal - Awaiting Review  
**Reviewers:** @arkavo-org/security-team @arkavo-org/platform-team  
**Labels:** `enhancement`, `security`, `configuration`, `opentdf`, `high-priority`