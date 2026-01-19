# KAS A2A Capability Runbook

Step-by-step guide to using the KAS (Key Access Service) A2A capability.

## Prerequisites

Build arkavo with the KAS feature:

```bash
cd /path/to/arkavo-edge
cargo build --features kas
```

## Step 1: Start the KAS Agent

```bash
cd examples/kas-a2a-capability
./launch.sh
```

Expected output:
```
KAS as A2A Capability Demo
===========================

Starting KAS-enabled agent...

Available JSON-RPC methods:
  - kas.publicKey  : Get KAS public key for TDF encryption
  - kas.rewrap     : Rewrap TDF keys with delegation verification

Endpoints:
  JSON-RPC : http://localhost:8080
  Agent Card: http://localhost:8081/.well-known/agent.json
```

## Step 2: Verify Agent Card

Check that KAS skills are advertised:

```bash
curl -s http://localhost:8081/.well-known/agent.json | jq '.skills'
```

Expected output includes:
```json
[
  {
    "id": "kas.rewrap",
    "name": "TDF Key Rewrap",
    "description": "Rewrap TDF encryption keys with ABAC policy enforcement",
    "tags": ["kas", "tdf", "encryption"]
  },
  {
    "id": "kas.publicKey",
    "name": "KAS Public Key",
    "description": "Get KAS public key for TDF encryption",
    "tags": ["kas", "crypto"]
  }
]
```

## Step 3: Get KAS Public Key

Request the KAS public key:

```bash
curl -s -X POST http://localhost:8080 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "kas.publicKey",
    "params": {}
  }' | jq
```

Expected response:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "public_key": "-----BEGIN PUBLIC KEY-----\n...\n-----END PUBLIC KEY-----",
    "key_id": "kas-demo-key-1",
    "algorithm": "ec:secp256r1"
  }
}
```

## Step 4: Test kas.rewrap (Without Handler)

Without a configured KAS handler, you'll get an error:

```bash
curl -s -X POST http://localhost:8080 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "kas.rewrap",
    "params": {
      "wrapped_key": "dGVzdC13cmFwcGVkLWtleQ==",
      "policy_binding": {"alg": "HS256", "hash": "dGVzdC1oYXNo"},
      "policy": "eyJhdHRyaWJ1dGVzIjpbXX0=",
      "delegation_token": "{}",
      "client_public_key": "-----BEGIN PUBLIC KEY-----\ntest\n-----END PUBLIC KEY-----"
    }
  }' | jq
```

Expected error response:
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "error": {
    "code": -32603,
    "message": "KAS capability not enabled",
    "data": "The KAS capability is not configured on this agent"
  }
}
```

## Step 5: Understanding Delegation Tokens

A valid delegation token chain looks like this:

```json
{
  "issuer_did": "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
  "subject_did": "did:key:z6MkpTHR8VNsBxYAAWHut2Geadd9jSwuBV8xRoAnwWsdvktH",
  "entitlements": [
    "https://arkavo.net/attr/role/value/admin",
    "https://arkavo.net/attr/clearance/value/secret"
  ],
  "expires_at": "2025-12-31T23:59:59Z",
  "signature": "BASE64_ED25519_SIGNATURE",
  "parent": null
}
```

The verifier will:
1. Check that `subject_did` matches the caller
2. Verify the Ed25519 signature using `issuer_did`
3. Check expiration
4. Recursively verify parent tokens
5. Confirm chain terminates at a trusted root

## Step 6: Understanding ABAC Policies

TDF policies specify required attributes:

```json
{
  "id": "policy-123",
  "attributes": [
    {
      "attribute": "https://arkavo.net/attr/role",
      "values": ["admin", "operator"]
    },
    {
      "attribute": "https://arkavo.net/attr/clearance",
      "values": ["secret", "top-secret"]
    }
  ],
  "dissemination": []
}
```

To satisfy this policy, the delegation token must have entitlements like:
- `https://arkavo.net/attr/role/value/admin` OR `https://arkavo.net/attr/role/value/operator`
- AND `https://arkavo.net/attr/clearance/value/secret` OR `https://arkavo.net/attr/clearance/value/top-secret`

## Error Codes Reference

| Code | Meaning | Resolution |
|------|---------|------------|
| -32001 | Access denied | Delegation token lacks required entitlements |
| -32002 | Delegation verification failed | Invalid signature, expired, or chain broken |
| -32003 | Invalid policy binding | Policy HMAC doesn't match |
| -32603 | KAS not configured | Enable KAS feature and configure handler |

## Cleanup

Stop the agent:

```bash
./stop.sh
```

## Next Steps

1. Configure a real KAS keypair for production use
2. Set up trusted root authorities
3. Integrate with TDF client libraries
4. See [arkavo-tdf](../../crates/arkavo-tdf/) for the implementation
