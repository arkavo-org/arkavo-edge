# arkavo-agent-identity

The Agent Identity Authority (AIA) — a dedicated role whose sole responsibility
is to issue and attest agent identity.

## Role boundary

The AIA is not an orchestrator and not an access policy decision point.

It **does**: create agent identities, issue credentials, rotate agent keys,
attest runtime/platform identity, publish a trust chain, and revoke compromised
agents.

It **does not**: assign work, grant resource access, make ABAC decisions,
delegate authority, or release TDF keys. Those remain the orchestrator's and
KAS's responsibilities. Keeping issuance separate from authorization prevents
the orchestrator from becoming a super-agent that can both mint identities and
grant access.

```text
Root Identity Agent (trust anchor)
   └─ Swarm Identity Agent (issues identities)   ← IdentityAuthority
        └─ Orchestrator Agent (delegates work)
             └─ Worker Agents
```

`Identity Agent ≈ OIDC IdP + SPIFFE CA`, `Orchestrator ≈ STS / capability
issuer`, `OpenTDF ≈ policy enforcement + key release`.

## OpenTDF attribute origin

`IdentityDocument::tdf_attributes()` emits only attributes that originate from
identity:

- `agent.identity.type`
- `agent.identity.trust_level`
- `agent.identity.runtime`
- `agent.identity.owner`
- `agent.identity.organization`

It never emits `ORCHESTRATOR_ISSUED_ATTRIBUTES` — `agent.tool_access`,
`agent.code_authority`, `agent.resource_scope`, `agent.execution_mode` — which
describe what an agent may *do* and are issued by the orchestrator.

## Trust, not self-assertion

A credential reaches `TrustLevel::Attested` only when an `IssueRequest` carries
attestation evidence (`arkavo-attestation` model or trusted-platform evidence).
Compromised-platform evidence is refused outright. The caller cannot assert a
trust level.

```rust
use arkavo_agent_identity::{IdentityAuthority, IssueRequest, Runtime};
use arkavo_crypto::AgentKeypair;
use chrono::{Duration, Utc};

let aia = IdentityAuthority::new_root("agent:id-authority");
let worker = AgentKeypair::generate();

let credential = aia.issue(IssueRequest {
    subject: "agent:worker-17".into(),
    agent_type: "coding".into(),
    agent_public_key: worker.public_key(),
    runtime: Runtime::Local,
    capabilities: vec!["a2a".into(), "mcp".into()],
    owner: None,
    organization: None,
    validity: Duration::days(10),
    attestation: None,
})?;

// A verifier resolves the issuer key from a trusted trust chain, then verifies.
let issuer_key = aia.trust_chain().resolve(&aia.verifying_key(), Utc::now())?;
let document = credential.verify(&issuer_key, Utc::now())?;
```

Credentials are Ed25519-signed over the canonical document bytes. Verification
requires an authority key resolved from a `TrustChain` anchored at a trusted
root — never the credential's own `iss` claim.
