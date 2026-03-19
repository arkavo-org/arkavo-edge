# Pillar 3: A2A over Iroh - Decentralized Agent Communication

## Executive Summary

Bridge **MCP** (local tool execution) with **A2A** (agent-to-agent communication) by tunneling A2A protocol over the **Iroh P2P mesh**. This enables agent collaboration in air-gapped, edge, and partitioned environments without centralized cloud brokers.

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         AGENT COMMUNICATION STACK                           │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│   LAYER 4: APPLICATION (A2A Protocol)                                       │
│   ┌─────────────────────────────────────────────────────────────────────┐   │
│   │  Agent2Agent (A2A) Standard                                          │   │
│   │  - Task delegation                                                   │   │
│   │  - Context sharing                                                   │   │
│   │  - Capability discovery                                              │   │
│   │  - Result return                                                     │   │
│   └─────────────────────────────────────────────────────────────────────┘   │
│                                   │                                          │
│   LAYER 3: TRANSPORT (Iroh QUIC)                                              │
│   ┌─────────────────────────────────────────────────────────────────────┐   │
│   │  Iroh P2P Mesh                                                       │   │
│   │  - Direct peer-to-peer connections                                   │   │
│   │  - NAT traversal (no cloud relay)                                    │   │
│   │  - Content-addressed data (blobs)                                    │   │
│   │  - DID-authenticated endpoints                                       │   │
│   └─────────────────────────────────────────────────────────────────────┘   │
│                                   │                                          │
│   LAYER 2: ENCRYPTION (OpenTDF)                                               │
│   ┌─────────────────────────────────────────────────────────────────────┐   │
│   │  Post-Quantum Resistant Encryption                                   │   │
│   │  - AES-256-GCM for data at rest                                      │   │
│   │  - Kyber/Dilithium for key exchange (future)                         │   │
│   │  - ABAC policy enforcement                                           │   │
│   │  - Per-message encryption keys                                       │   │
│   └─────────────────────────────────────────────────────────────────────┘   │
│                                   │                                          │
│   LAYER 1: IDENTITY (DID)                                                     │
│   ┌─────────────────────────────────────────────────────────────────────┐   │
│   │  Decentralized Identifiers                                           │   │
│   │  - did:arkavo:<agent-id>                                             │   │
│   │  - Device-bound keys                                                 │   │
│   │  - Cryptographic attestation                                         │   │
│   └─────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘


┌─────────────────────────────────────────────────────────────────────────────┐
│                      MESH TOPOLOGY EXAMPLES                                  │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  SCENARIO 1: Air-Gapped Industrial Control                                  │
│  ┌──────────┐      ┌──────────┐      ┌──────────┐                          │
│  │ Sensor   │◄────►│ Edge     │◄────►│ Control  │                          │
│  │ Agent    │ Iroh │ Gateway  │ Iroh │ Room PC  │                          │
│  │ (Iroh)   │ P2P  │ Agent    │ P2P  │ (HITL)   │                          │
│  └──────────┘      └──────────┘      └──────────┘                          │
│       ▲                                    │                                 │
│       │ No internet required!              │ Manual approval                │
│       │                                    │ for critical                   │
│       └────────────────────────────────────┘ actions                        │
│                                                                              │
│  SCENARIO 2: Multi-Site Enterprise                                          │
│  ┌──────────┐         ┌──────────┐         ┌──────────┐                     │
│  │ HQ       │◄───────►│ Iroh     │◄───────►│ Branch   │                     │
│  │ Conductor│   P2P   │ Relay    │   P2P   │ Office   │                     │
│  │ Agent    │ (opt)   │ (opt)    │ (opt)   │ Agent    │                     │
│  └────┬─────┘         └──────────┘         └────┬─────┘                     │
│       │                                          │                            │
│       └──────────────────┬───────────────────────┘                            │
│                          │ Direct P2P preferred                               │
│                          ▼                                                   │
│                   ┌──────────────┐                                           │
│                   │ Remote       │                                           │
│                   │ Worker Agent │                                           │
│                   └──────────────┘                                           │
│                                                                              │
│  SCENARIO 3: Edge Compute Swarm                                             │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐                                   │
│  │ IoT      │  │ IoT      │  │ IoT      │                                   │
│  │ Device 1 │◄►│ Device 2 │◄►│ Device 3 │                                   │
│  │ (Agent)  │  │ (Agent)  │  │ (Agent)  │                                   │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘                                   │
│       │             │             │                                          │
│       └─────────────┼─────────────┘                                          │
│                     ▼                                                        │
│               ┌──────────┐                                                   │
│               │ Swarm    │                                                   │
│               │ Leader   │                                                   │
│               └──────────┘                                                   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

## A2A over Iroh Protocol

### Message Flow

```rust
// crates/arkavo-a2a/src/iroh_transport.rs

/// A2A message wrapped for Iroh transport
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrohA2aMessage {
    /// A2A protocol version
    pub version: String,
    
    /// Sender DID
    pub from: String,
    
    /// Recipient DID
    pub to: String,
    
    /// Message type
    pub message_type: A2aMessageType,
    
    /// Encrypted payload (OpenTDF)
    pub payload: TdfEncryptedBlob,
    
    /// Sender signature
    pub signature: Vec<u8>,
    
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    
    /// Message ID (for deduplication)
    pub message_id: String,
    
    /// Reply-to (for async responses)
    pub reply_to: Option<String>,
}

/// A2A transport over Iroh
pub struct IrohA2aTransport {
    /// Iroh node
    iroh: IrohNode,
    
    /// Our DID
    did: String,
    
    /// OpenTDF encryptor
    tdf: OpenTdfService,
    
    /// Message handlers
    handlers: DashMap<A2aMessageType, Box<dyn A2aHandler>>,
}

impl IrohA2aTransport {
    /// Send A2A message to another agent
    pub async fn send_message(
        &self,
        to: &str,
        message: A2aMessage,
    ) -> Result<MessageId, A2aError> {
        // 1. Resolve recipient DID to Iroh node address
        let recipient_addr = self.resolve_did_to_iroh(to).await?;
        
        // 2. Build ABAC policy for this message
        let policy = self.build_message_policy(&message, to)?;
        
        // 3. Encrypt message with OpenTDF
        let payload = self.tdf.encrypt(
            &serde_json::to_vec(&message)?,
            &policy,
        ).await?;
        
        // 4. Sign the message
        let signature = self.sign_message(&payload)?;
        
        // 5. Build Iroh-wrapped message
        let iroh_message = IrohA2aMessage {
            version: "1.0".to_string(),
            from: self.did.clone(),
            to: to.to_string(),
            message_type: message.type_(),
            payload,
            signature,
            timestamp: Utc::now(),
            message_id: generate_message_id(),
            reply_to: None,
        };
        
        // 6. Send via Iroh QUIC
        self.send_quic(recipient_addr, &iroh_message).await?;
        
        // 7. Audit log
        self.audit.log_outbound(&iroh_message).await?;
        
        Ok(iroh_message.message_id)
    }
    
    /// Receive and process incoming message
    pub async fn receive_message(
        &self,
        message: IrohA2aMessage,
    ) -> Result<(), A2aError> {
        // 1. Verify signature
        if !self.verify_signature(&message)? {
            return Err(A2aError::InvalidSignature);
        }
        
        // 2. Check A2A policy (is sender allowed to send this type?)
        if !self.check_a2a_policy(&message).await? {
            return Err(A2aError::PolicyDenied);
        }
        
        // 3. Decrypt payload
        let decrypted = self.tdf.decrypt(&message.payload).await?;
        let a2a_message: A2aMessage = serde_json::from_slice(&decrypted)?;
        
        // 4. Route to handler
        if let Some(handler) = self.handlers.get(&message.message_type) {
            handler.handle(a2a_message).await?;
        }
        
        // 5. Audit log
        self.audit.log_inbound(&message).await?;
        
        Ok(())
    }
    
    /// Resolve DID to Iroh node address
    async fn resolve_did_to_iroh(&self, did: &str) -> Result<NodeAddr, A2aError> {
        // 1. Resolve DID Document
        let doc = did_resolver::resolve(did).await?;
        
        // 2. Extract Iroh service endpoint
        let iroh_service = doc.service
            .iter()
            .find(|s| s.type_ == "IrohNode")
            .ok_or(A2aError::NoIrohEndpoint)?;
        
        // 3. Parse Iroh address
        let node_id = NodeId::from_str(&iroh_service.service_endpoint)?;
        
        // 4. Build NodeAddr (with known relays if needed)
        let addr = NodeAddr::from(node_id)
            .with_relay_urls(relay_urls);
        
        Ok(addr)
    }
}
```

### DID Document with Iroh Service

```json
{
  "@context": ["https://www.w3.org/ns/did/v1"],
  "id": "did:arkavo:agent-123",
  "verificationMethod": [{
    "id": "did:arkavo:agent-123#keys-1",
    "type": "Ed25519VerificationKey2020",
    "controller": "did:arkavo:agent-123",
    "publicKeyMultibase": "z6Mkq..."
  }],
  "service": [{
    "id": "did:arkavo:agent-123#iroh",
    "type": "IrohNode",
    "serviceEndpoint": "pq3p2q3p4q5p6p7p8p9papbpc..."
  }, {
    "id": "did:arkavo:agent-123#kas",
    "type": "OpenTdfKas",
    "serviceEndpoint": "https://kas.agent-123.local"
  }]
}
```

## Conductor-Specialist Pattern over Iroh

### Task Delegation Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    CONDUCTOR-SPECIALIST DELEGATION                          │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Phase 1: Task Assignment                                                    │
│  ═══════════════════════                                                     │
│                                                                              │
│  ┌─────────────┐                              ┌─────────────┐              │
│  │  Conductor  │                              │  Specialist │              │
│  │   Agent     │                              │   Agent     │              │
│  │             │   1. SEND TASK (A2A over     │             │              │
│  │  ┌───────┐  │      Iroh + OpenTDF)         │  ┌───────┐  │              │
│  │  │Task   │──┼──────────────────────────────►│  │Task   │  │              │
│  │  │Queue  │  │      Encrypted with:         │  │Queue  │  │              │
│  │  └───────┘  │      - recipient DID         │  └───────┘  │              │
│  │      │      │      - task classification   │      │      │              │
│  │      ▼      │      - TTL constraints       │      ▼      │              │
│  │  ┌───────┐  │                              │  ┌───────┐  │              │
│  │  │Iroh   │  │                              │  │Iroh   │  │              │
│  │  │Node   │  │                              │  │Node   │  │              │
│  │  └───┬───┘  │                              │  └───┬───┘  │              │
│  └──────┼──────┘                              └──────┼──────┘              │
│         │                                            │                       │
│         └──────────────► Iroh P2P Mesh ◄─────────────┘                       │
│                                                                              │
│  Phase 2: Context Sharing                                                    │
│  ════════════════════════                                                    │
│                                                                              │
│  ┌─────────────┐                              ┌─────────────┐              │
│  │  Conductor  │                              │  Specialist │              │
│  │             │   2. SHARE CONTEXT           │             │              │
│  │  ┌───────┐  │      (Iroh Blob Transfer)    │  ┌───────┐  │              │
│  │  │Context│──┼──────────────────────────────►│  │Context│  │              │
│  │  │Store  │  │                              │  │Store  │  │              │
│  │  └───┬───┘  │      Large context sent      │  └───┬───┘  │              │
│  │      │      │      as Iroh blob:           │      │      │              │
│  │  ┌───▼───┐  │      - Content-addressed     │  ┌───▼───┐  │              │
│  │  │Iroh   │  │      - Encrypted (TDF)       │  │Iroh   │  │              │
│  │  │Blob   │  │      - Ticket-based access   │  │Blob   │  │              │
│  │  └───┬───┘  │                              │  └───┬───┘  │              │
│  └──────┼──────┘                              └──────┼──────┘              │
│         │                                            │                       │
│         └──────────────► Iroh P2P Mesh ◄─────────────┘                       │
│                                                                              │
│  Phase 3: Result Return                                                      │
│  ═══════════════════════                                                     │
│                                                                              │
│  ┌─────────────┐                              ┌─────────────┐              │
│  │  Conductor  │                              │  Specialist │              │
│  │             │   3. RETURN RESULT           │             │              │
│  │  ┌───────┐  │      (A2A over Iroh)         │  ┌───────┐  │              │
│  │  │Result │◄─┼──────────────────────────────│──┤Result │  │              │
│  │  │Queue  │  │                              │  │Queue  │  │              │
│  │  └───────┘  │      Result includes:        │  └───────┘  │              │
│  │             │      - task_id               │             │              │
│  │             │      - status                │             │              │
│  │             │      - output (encrypted)    │             │              │
│  │             │      - execution proof       │             │              │
│  │             │      - resource usage        │             │              │
│  └─────────────┘                              └─────────────┘              │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Iroh Blob Sharing for Large Context

```rust
// crates/arkavo-a2a/src/context_share.rs

/// Share large context via Iroh blobs
pub struct ContextSharing {
    iroh: IrohNode,
    tdf: OpenTdfService,
}

impl ContextSharing {
    /// Export context as Iroh blob with TDF encryption
    pub async fn export_context(
        &self,
        context: &Context,
        allowed_recipients: &[String], // DIDs
    ) -> Result<IrohTicket, ContextError> {
        // 1. Serialize context
        let data = serialize_context(context)?;
        
        // 2. Build ABAC policy for recipients
        let policy = PolicyBuilder::new()
            .attribute(
                "https://arkavo.net/attr/agent-id",
                &allowed_recipients.iter()
                    .map(|d| d.strip_prefix("did:arkavo:").unwrap_or(d))
                    .collect::<Vec<_>>(),
            )
            .attribute(
                "https://arkavo.net/attr/context-access",
                &["read"],
            )
            .build()?;
        
        // 3. Encrypt with OpenTDF
        let encrypted = self.tdf.encrypt(&data, &policy).await?;
        
        // 4. Import to Iroh as blob
        let blob = self.iroh.blobs().add_bytes(encrypted).await?;
        
        // 5. Create ticket for sharing
        let ticket = IrohTicket {
            blob_hash: blob.hash,
            size: blob.size,
            policy: policy.clone(),
            expiration: Utc::now() + Duration::hours(24),
        };
        
        Ok(ticket)
    }
    
    /// Import context from Iroh ticket
    pub async fn import_context(
        &self,
        ticket: &IrohTicket,
        our_did: &str,
    ) -> Result<Context, ContextError> {
        // 1. Fetch blob from Iroh mesh
        let blob = self.iroh.blobs()
            .fetch(ticket.blob_hash)
            .await?;
        
        // 2. Decrypt with OpenTDF
        // KAS will verify our DID against ticket policy
        let decrypted = self.tdf.decrypt(&blob.data).await?;
        
        // 3. Deserialize context
        let context = deserialize_context(&decrypted)?;
        
        Ok(context)
    }
}

/// Iroh ticket for context sharing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrohTicket {
    /// Blob hash (content address)
    pub blob_hash: [u8; 32],
    
    /// Blob size
    pub size: u64,
    
    /// Access policy
    pub policy: Policy,
    
    /// Expiration time
    pub expiration: DateTime<Utc>,
}
```

## MCP vs A2A Responsibilities

| Aspect | MCP (Model Context Protocol) | A2A over Iroh |
|--------|------------------------------|---------------|
| **Scope** | Local tool execution | Inter-agent communication |
| **Transport** | Stdio/SSE (local) | Iroh QUIC P2P (network) |
| **Security** | OS sandboxing | OpenTDF + DID auth |
| **Use Case** | File read, shell exec | Task delegation, context sharing |
| **Latency** | < 10ms | < 50ms (P2P) |
| **Offline** | N/A | ✓ Works air-gapped |
| **Encryption** | N/A (local) | ✓ OpenTDF per-message |

### Integration Point

```rust
// crates/arkavo-cli/src/agent_bridge.rs

/// Bridge between MCP (local) and A2A (mesh)
pub struct AgentBridge {
    /// Local MCP client
    mcp: McpClient,
    
    /// A2A over Iroh transport
    a2a: IrohA2aTransport,
    
    /// Our agent identity
    identity: AgentIdentity,
}

impl AgentBridge {
    /// Execute tool - decide MCP or A2A
    pub async fn execute(&self, request: ToolRequest) -> Result<ToolResult, Error> {
        match request.target {
            ToolTarget::Local => {
                // Use MCP for local tools
                self.mcp.execute(request).await
            }
            ToolTarget::Remote(agent_did) => {
                // Use A2A for remote agents
                let task = A2aTask::from(request);
                let result = self.a2a
                    .delegate_task(&agent_did, task)
                    .await?;
                Ok(result.into())
            }
        }
    }
}
```

## Implementation Roadmap

### Phase 1: Iroh Integration (Weeks 1-4)

```rust
// New crate: arkavo-a2a

pub mod iroh_transport;
pub mod context_share;
pub mod delegation;

// Implement:
// - Iroh node initialization
// - DID-to-Iroh address resolution
// - Basic message transport
```

### Phase 2: OpenTDF Integration (Weeks 5-8)

```rust
// Extend arkavo-a2a

// Implement:
// - Per-message OpenTDF encryption
// - ABAC policy enforcement
// - TDF audit logging
```

### Phase 3: A2A Protocol (Weeks 9-12)

```rust
// Implement A2A standard:
// - Task delegation
// - Capability discovery
// - Result return
// - Context sharing via Iroh blobs
```

### Phase 4: Mesh Orchestration (Weeks 13-16)

```rust
// Conductor-specialist implementation:
// - Task routing
// - Load balancing
// - Failure recovery
// - HITL coordination
```

## Competitive Advantages

| Feature | Centralized A2A (HTTP) | Arkavo A2A over Iroh |
|---------|----------------------|----------------------|
| **Offline/Air-gapped** | ✗ Requires internet | ✓ P2P direct |
| **Latency** | 100-500ms (cloud) | < 50ms (local P2P) |
| **Privacy** | Data through cloud | Direct encryption |
| **Censorship Resistance** | Cloud can block | P2P mesh routing |
| **Infrastructure Cost** | $$$ Cloud brokers | $ Edge devices |
| **Encryption** | TLS (transport) | OpenTDF (per-message) |
| **Identity** | API keys | DIDs + VCs |
