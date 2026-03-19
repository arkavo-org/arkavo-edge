# Pillar 1: ADL & CoSAI-Unified Policy Engine

## Executive Summary

Transform the `AgentPolicy.yaml` schema into a standards-compliant policy engine that serves as the "border patrol" for ADL-defined agents, with explicit CoSAI/OWASP traceability.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     EXTERNAL POLICY SOURCES                      │
├─────────────────────────────────────────────────────────────────┤
│  ADL (Agent Definition Language)    CoSAI Secure Design Patterns │
│  ┌─────────────┐                    ┌──────────────────────┐    │
│  │ Agent Spec  │                    │ Threat Matrix        │    │
│  │ - Identity  │                    │ - Mitigation IDs     │    │
│  │ - Capabilities│                  │ - Controls           │    │
│  │ - Constraints│                   └──────────────────────┘    │
│  └──────┬──────┘                                                 │
│         │         OWASP ASI 2026                                 │
│         │         ┌──────────────────┐                          │
│         └────────►│ ASI01-ASI10      │                          │
│                   └──────────────────┘                          │
└──────────────────────┬──────────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────────┐
│              ARKAVO POLICY UNIFICATION LAYER                     │
│              (Rust-native, OPA-Wasm compatible)                  │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │ ADL Parser   │  │ CoSAI Mapper │  │ OWASP ASI Evaluator  │  │
│  │              │  │              │  │                      │  │
│  │ Converts ADL │  │ Maps threats │  │ Validates against    │  │
│  │ to internal  │  │ to controls  │  │ ASI Top 10           │  │
│  │ policy spec  │  │              │  │                      │  │
│  └──────┬───────┘  └──────┬───────┘  └──────────┬───────────┘  │
│         │                 │                     │              │
│         └─────────────────┼─────────────────────┘              │
│                           │                                    │
│                           ▼                                    │
│                  ┌─────────────────┐                           │
│                  │ Unified Policy  │                           │
│                  │ Graph (OPA/     │                           │
│                  │ Custom Rust)    │                           │
│                  └────────┬────────┘                           │
│                           │                                    │
│                  ┌────────▼────────┐                           │
│                  │ Rego/Wasm       │                           │
│                  │ Evaluator       │                           │
│                  │ < 1ms latency   │                           │
│                  └────────┬────────┘                           │
│                           │                                    │
└───────────────────────────┼────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                    ENFORCEMENT POINTS                            │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐ │
│  │ MCP Tool    │  │ A2A Comms   │  │ Data Access (OpenTDF)   │ │
│  │ Invocation  │  │             │  │                         │ │
│  │             │  │             │  │                         │ │
│  │ Policy check│  │ Policy check│  │ ABAC evaluation         │ │
│  │ + audit log │  │ + audit log │  │ + audit log             │ │
│  │ (CoSAI-WS4) │  │ (ASI07)     │  │ (ASI06)                 │ │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘ │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## ADL Compatibility Layer

### ADL-to-Arkavo Mapping

```rust
// crates/arkavo-policy/src/adl.rs

use serde::{Deserialize, Serialize};

/// Agent Definition Language (ADL) specification
/// Reference: https://github.com/DIF-TAAWG/ADL-Spec
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdlAgentDefinition {
    /// Agent identity (DID)
    pub id: String,
    
    /// Agent version
    pub version: String,
    
    /// Agent capabilities (what it CAN do)
    pub capabilities: Vec<AdlCapability>,
    
    /// Agent constraints (what it MUST NOT do)
    pub constraints: Vec<AdlConstraint>,
    
    /// Delegation rules
    pub delegation: Option<AdlDelegation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdlCapability {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub resources: Vec<String>,
    pub max_risk_level: RiskLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdlConstraint {
    pub type_: String, // "constitutional", "operational", "regulatory"
    pub description: String,
    pub enforcement: EnforcementLevel,
}

impl AdlAgentDefinition {
    /// Convert ADL definition to Arkavo internal policy
    pub fn to_arkavo_policy(&self) -> UnifiedPolicy {
        UnifiedPolicy {
            agent_id: self.id.clone(),
            
            // Map ADL capabilities to tool allowlist
            tool_allowlist: self.capabilities.iter()
                .flat_map(|c| c.tools.clone())
                .collect(),
            
            // Map ADL constraints to constitutional principles
            constitutional_principles: self.constraints.iter()
                .filter(|c| c.type_ == "constitutional")
                .map(|c| ConstitutionalPrinciple {
                    id: format!("adl-{}", uuid::Uuid::new_v4()),
                    principle: c.description.clone(),
                    immutable: true,
                    source: "ADL".to_string(),
                })
                .collect(),
            
            // Map risk levels
            max_risk_level: self.capabilities.iter()
                .map(|c| c.max_risk_level.clone())
                .max()
                .unwrap_or(RiskLevel::Low),
        }
    }
}
```

## CoSAI Integration

### CoSAI Workstream 4 Mapping

| CoSAI Control | Arkavo Implementation | Audit Log Tag |
|--------------|----------------------|---------------|
| **SD-001**: Input Validation | Preflight Moderator (TØR-G circuits) | `COSAI-SD-001` |
| **SD-002**: Sandboxing | eBPF/Wasm sandboxing | `COSAI-SD-002` |
| **SD-003**: Least Privilege | ABAC tool bindings | `COSAI-SD-003` |
| **SD-004**: Secure Defaults | Deny-by-default A2A policy | `COSAI-SD-004` |
| **SD-005**: Audit Logging | TDF-encrypted audit trail | `COSAI-SD-005` |
| **SD-006**: Resource Limits | Token/budget/TTL controls | `COSAI-SD-006` |

### Audit Log Format with CoSAI Tags

```json
{
  "timestamp": "2026-03-17T17:53:34Z",
  "event_type": "tool_execution_denied",
  "agent_id": "did:arkavo:agent-123",
  "mitigations": [
    {
      "framework": "OWASP-ASI",
      "id": "ASI02",
      "description": "Tool Misuse & Exploitation"
    },
    {
      "framework": "CoSAI-WS4",
      "id": "SD-003",
      "description": "Least Privilege Violation"
    }
  ],
  "context": {
    "tool": "shell_exec",
    "risk_level": "critical",
    "required_entitlements": ["role/admin"],
    "actual_entitlements": ["role/user"]
  },
  "tdf_manifest": "<encrypted_audit_blob>"
}
```

## OPA-Wasm Policy Engine

### Implementation Strategy

```rust
// crates/arkavo-policy/src/engine.rs

use wasmtime::{Engine, Module, Store, Instance};

/// Open Policy Agent (OPA) WebAssembly policy engine
pub struct OpaPolicyEngine {
    engine: Engine,
    module: Module,
    // Cache for compiled policies
    policy_cache: DashMap<String, CompiledPolicy>,
}

impl OpaPolicyEngine {
    pub fn new() -> Result<Self, PolicyError> {
        let engine = Engine::default();
        // Compile OPA Wasm runtime
        let module = Module::from_file(&engine, "opa-runtime.wasm")?;
        
        Ok(Self {
            engine,
            module,
            policy_cache: DashMap::new(),
        })
    }
    
    /// Evaluate policy with < 1ms latency target
    pub fn evaluate(&self, input: &PolicyInput) -> Result<PolicyDecision, PolicyError> {
        let start = Instant::now();
        
        // Load compiled policy from cache
        let policy = self.policy_cache
            .get(&input.policy_id)
            .ok_or(PolicyError::PolicyNotFound)?;
        
        // Create Wasm instance
        let mut store = Store::new(&self.engine, ());
        let instance = Instance::new(&mut store, &self.module, &[])?;
        
        // Call OPA eval function
        let eval = instance.get_typed_func::<(i32, i32), i32>(&mut store, "eval")?;
        
        // Serialize input to Wasm memory
        let input_json = serde_json::to_vec(&input)?;
        let result = eval.call(&mut store, (input_ptr, input_len))?;
        
        // Parse result
        let decision: PolicyDecision = self.parse_result(&store, result)?;
        
        // Ensure < 1ms latency
        let elapsed = start.elapsed();
        if elapsed > Duration::from_micros(1000) {
            tracing::warn!("Policy evaluation exceeded 1ms: {:?}", elapsed);
        }
        
        Ok(decision)
    }
}
```

### Custom Rust Parser (Alternative to OPA)

For ultra-low latency (< 100μs), implement a custom Rust policy parser:

```rust
// crates/arkavo-policy/src/fast_eval.rs

/// Zero-allocation policy evaluator for hot paths
pub struct FastPolicyEvaluator;

impl FastPolicyEvaluator {
    /// Evaluate tool execution permission
    /// Target: < 100 microseconds
    pub fn can_execute_tool(
        &self,
        agent: &AgentIdentity,
        tool: &ToolRequest,
        policy: &UnifiedPolicy,
    ) -> PolicyDecision {
        // 1. Check tool allowlist (O(1) hash lookup)
        if !policy.tool_allowlist.contains(&tool.name) {
            return PolicyDecision::Deny {
                reason: "Tool not in allowlist".to_string(),
                mitigations: vec![
                    Mitigation::cosai("SD-003"),
                    Mitigation::owasp_as("ASI02"),
                ],
            };
        }
        
        // 2. Check risk level (integer comparison)
        if tool.risk_level > policy.max_risk_level {
            return PolicyDecision::Deny {
                reason: "Risk level exceeds agent clearance".to_string(),
                mitigations: vec![
                    Mitigation::cosai("SD-006"),
                    Mitigation::owasp_as("ASI02"),
                ],
            };
        }
        
        // 3. Check ABAC entitlements
        if !self.check_entitlements(agent, tool, policy) {
            return PolicyDecision::Deny {
                reason: "Insufficient entitlements".to_string(),
                mitigations: vec![
                    Mitigation::cosai("SD-003"),
                    Mitigation::owasp_as("ASI02"),
                ],
            };
        }
        
        PolicyDecision::Allow
    }
}
```

## Integration with Existing Arkavo Crates

```
crates/
├── arkavo-policy/              # NEW: Unified policy engine
│   ├── src/
│   │   ├── lib.rs
│   │   ├── adl.rs             # ADL parsing
│   │   ├── cosai.rs           # CoSAI mappings
│   │   ├── engine.rs          # OPA-Wasm engine
│   │   ├── fast_eval.rs       # Ultra-fast Rust evaluator
│   │   └── audit.rs           # Standards-compliant logging
│   └── tests/
│       ├── adl_compat_test.rs
│       └── cosai_mapping_test.rs
│
├── arkavo-orchestrator/
│   └── src/task_policy_manager.rs  # Refactor to use arkavo-policy
│
├── arkavo-router/
│   └── src/preflight/moderator.rs  # Add CoSAI tags
│
└── arkavo-protocol/
    └── src/a2a_policy.rs      # Export ADL-compatible policies
```

## Migration Path

### Phase 1: Schema Alignment (Weeks 1-2)

1. Extend `AgentPolicy.yaml` with ADL-compatible fields
2. Add CoSAI threat mapping to policy schema
3. Create migration tool from existing policies

### Phase 2: Engine Implementation (Weeks 3-4)

1. Implement OPA-Wasm runtime
2. Create fast Rust evaluator for hot paths
3. Build ADL parser

### Phase 3: Integration (Weeks 5-6)

1. Migrate `TaskPolicyManager` to use new engine
2. Update audit logging with CoSAI/OWASP tags
3. Add policy hot-reload support

### Phase 4: Compliance Export (Weeks 7-8)

1. Build SIEM integration plugins
2. Create compliance dashboard
3. Publish CoSAI/OWASP compliance documentation
