# TDF-JSON binding for A2A

**Status:** design, 2026-10-26. **Builds on:** the workload key, key statements and signed agent card of #766 (#772–#784). **Implements:** the sealed envelope of [the swarm trust model](../swarm-trust-model.md) ("Agent ingress, discovery and enrollment") on [A2A 1.0.0](https://a2a-protocol.org/v1.0.0/specification/). **Changes:** WS2, WS3 and WS5 of [the realignment scope](realignment-scope.md).

Edge speaks standard A2A and seals it with TDF. Both pieces below use TDF-JSON, the inline JSON container of TDF-JSON draft-00 that `opentdf::jsonrpc::TdfJson` in opentdf-rs 0.16.1 implements:

| Piece | Seals | Opened by | A2A mechanism |
|---|---|---|---|
| TDF-JSON binding | The whole JSON-RPC message: every member request, reply, stream event and gossip message | The agent or swarm the message is addressed to | A custom protocol binding in the card's `supportedInterfaces` |
| TDF-JSON Part | One piece of content inside a message | Whoever the content's own policy releases it to, which need not be the recipient | A `Part` with `mediaType: application/tdf+json`, declared by a profile extension |

The binding is not optional: an agent offers peers no unsealed interface. The Part is the option, for content whose audience differs from the message's.

## Why a binding and not an extension

An A2A extension may add `metadata`, card parameters or methods; it [must not change core data structures](https://a2a-protocol.org/v1.0.0/topics/extensions/). Sealing the whole message changes the body, which A2A leaves to [protocol bindings](https://a2a-protocol.org/v1.0.0/topics/custom-protocol-bindings/): a binding carries every core operation, with the same meaning, over a different wire. So the envelope is a binding. Inside it the message is unmodified A2A 1.0 JSON-RPC (`SendMessage`, `SendStreamingMessage`, `GetTask` and the rest, with A2A's error codes), and a standard `RequestHandler` serves it without knowing it was sealed.

Arkavo defines only the binding, the envelope assertion, and two card extensions: `https://arkavo.com/a2a/key-statements/v1`, which #781 already serves, and `https://arkavo.com/a2a/tdf-json-part/v1`.

## Agent card

```json
{
  "supportedInterfaces": [
    {
      "url": "ws://agent.local:8443/a2a",
      "protocolBinding": "https://arkavo.com/a2a/bindings/tdf-json/v1",
      "protocolVersion": "1.0"
    }
  ],
  "capabilities": {
    "streaming": true,
    "extensions": [
      {
        "uri": "https://arkavo.com/a2a/key-statements/v1",
        "description": "Key statements the workload key signs for this agent's subordinate keys",
        "required": false,
        "params": { "statements": ["eyJhbGciOiJFUzI1NiJ9.e30.c2ln"] }
      },
      {
        "uri": "https://arkavo.com/a2a/tdf-json-part/v1",
        "description": "Accepts content sealed as a TDF-JSON Part",
        "required": false,
        "params": { "formats": ["json"] }
      }
    ]
  },
  "signatures": [{ "protected": "eyJhbGciOiJFUzI1NiJ9", "signature": "c2ln" }]
}
```

- **The card is the one public A2A object.** A stranger fetches it from `/.well-known/agent-card.json`; it carries only what a stranger may know. Members get the extended card through `GetExtendedAgentCard`, sealed.
- **The workload key signs it** with A2A's own `signatures` (a JWS over the RFC 8785 form), as SRV-017 does today.
- **The TDF key a sender seals to comes from the key-statements extension.** The sender checks that statement against the digest in the mDNS TXT record or QR code, and against the card's signer. Before enrollment the card leaves the extension out, so it names no DID (INGRESS-018).
- **Today's card is the pre-1.0 shape.** `AgentCard` in `crates/arkavo-protocol/src/types.rs` holds extensions at the top level with a `data` field and has no `supportedInterfaces`. The card above is the 1.0 shape that the realignment's card producer emits (WS2).

## The sealed document

Every message on the binding is one TDF-JSON document:

```json
{
  "tdf": "json",
  "version": "1.0.0",
  "manifest": {
    "encryptionInformation": {
      "type": "split",
      "keyAccess": [
        {
          "type": "wrapped",
          "url": "did:key:zDnaeRecipientWorkloadKey",
          "kid": "statement-digest-of-recipient-tdf-key",
          "protocol": "kas",
          "wrappedKey": "d3JhcHBlZA",
          "ephemeralPublicKey": "-----BEGIN PUBLIC KEY-----",
          "policyBinding": { "alg": "HS256", "hash": "YmluZGluZw" }
        }
      ],
      "method": { "algorithm": "AES-256-GCM", "isStreamable": true, "iv": "aXY" },
      "integrityInformation": {},
      "policy": "cG9saWN5"
    },
    "assertions": [
      {
        "id": "a2a-envelope",
        "type": "other",
        "scope": "tdo",
        "appliesToState": "encrypted",
        "statement": {
          "format": "json-structured",
          "schema": "https://arkavo.com/a2a/envelope/v1",
          "value": {
            "typ": "msg",
            "iss": "did:key:zDnaeSenderWorkloadKey",
            "aud": "did:key:zDnaeRecipientWorkloadKey",
            "iat": 1793000000,
            "exp": 1793000060,
            "nonce": "bm9uY2U",
            "stmt": "statement-digest-of-sender-tdf-key"
          }
        },
        "binding": { "method": "jws", "signature": "eyJhbGciOiJFUzI1NiJ9.e30.c2ln" }
      }
    ]
  },
  "payload": {
    "type": "inline",
    "protocol": "base64",
    "mimeType": "application/json",
    "isEncrypted": true,
    "value": "Y2lwaGVydGV4dA"
  }
}
```

The decrypted payload is:

```json
{
  "iss": "did:key:zDnaeSenderWorkloadKey",
  "nonce": "bm9uY2U",
  "statement": "eyJhbGciOiJFUzI1NiJ9.e30.c2ln",
  "serviceParameters": { "a2a-version": "1.0", "a2a-extensions": "https://arkavo.com/a2a/tdf-json-part/v1" },
  "rpc": { "jsonrpc": "2.0", "id": 1, "method": "SendMessage", "params": {} }
}
```

- **The signature is a TDF assertion.** The document carries exactly one assertion with id `a2a-envelope` and the envelope schema. Its `binding` is a compact JWS: header `{"alg":"ES256","typ":"a2a-envelope","kid":"<iss key id>"}`, payload `{"digest":"<b64url SHA-256>"}`. The digest is over the RFC 8785 canonical form of the whole document with that one `binding` member removed. The signature therefore covers the envelope fields, every key access object, the policy, the integrity information and the ciphertext, which meets OpenTDF's rule that a binding cover [the assertion and enough TDF context](https://github.com/opentdf/spec/blob/main/schema/OpenTDF/assertion_binding.md). The workload key makes it, in the Secure Enclave where there is one, with the same algorithm and `kid` form as key statements and card signatures. The distinct `typ` keeps an envelope signature from being read as a statement, a card or any other JWS the workload key makes.
- **The envelope fields are those of the trust model.** `typ` is `msg` or `pair`. `iss` is the sender's workload `did:key`, so verification needs no lookup. `aud` is the recipient's `did:key`, or a swarm it belongs to for gossip. `iat`, `exp` and `nonce` bound replay. `stmt` is the digest of the sender's TDF key statement, which a reply is sealed to. A reply or stream event adds `re`, the request's nonce, and a stream event adds `seq`.
- **A statement digest** is the base64url SHA-256 of the statement's compact JWS. The TXT record, the QR code, `stmt` and `kid` all use this form.
- **Key access says who can open it.** For a message addressed to one agent, the DEK is wrapped to that agent's TDF key, by ECDH P-256 with an ephemeral key, HKDF and AES-GCM as `TdfJson` wraps to a KAS key. Its `url` is the recipient's `did:key` and its `kid` the digest of the statement that names the key. That key is the agent's kept KAS key (TDFS-017), so the recipient unwraps with it locally without a request. For a swarm-addressed message, the key access object names the swarm's KAS, and the key is released under the status lease ("Key release"). The policy's `dissem` names the recipient's `did:key` or the swarm.
- **The sealed body repeats `iss` and `nonce`.** A receiver that finds them different from the signed values refuses the message, so one sender's ciphertext cannot be re-signed by another. The body also carries the sender's full TDF key statement, so the reply needs no fetch. It carries the A2A service parameters too, where A2A's guidance for carriers without headers puts them, so neither the version nor the extensions in use show on the wire.
- **`created` is omitted,** since `iat` is the signed time.

## Carriers

The binding carries whole documents and trusts no carrier. A WebSocket text frame, an HTTP POST body, an SSE `data:` line or a frame on iroh's `arkavo/a2a/1` ALPN each carry one document. Plain `ws` is acceptable (ADR-0037 §4): the carrier protects nothing that the document does not.

A receiver tells the three kinds of traffic apart by shape before unwrapping anything. A top-level `jsonrpc` member is a public request, answered only for `health` and, until the realignment lands, the legacy `agent_card` method. A `tdf` member equal to `json` is an envelope, and the `typ` inside its assertion says whether it is pairing. Anything else is refused.

## Receiving

A receiver processes a document in this order and stops at the first failure:

1. Bound the size, then parse. Check the shape: TDF-JSON `1.0.0`, inline base64 payload, exactly one `a2a-envelope` assertion.
2. Verify the JWS: `alg` ES256, `typ` `a2a-envelope`, a `kid` naming `iss`'s key, a valid signature, and a digest that matches the document as received.
3. Check that `aud` names this agent or a swarm it belongs to, that `exp` has not passed, that `iat` is within the clock bound, and that the nonce has not been seen within the window.
4. Establish the sender's standing: enrolled member or not, its state under "State overrides score", and the local deny latch. A member envelope from a non-member is refused here. A pairing envelope passes only while a pairing window is open.
5. Only then unwrap. Pick the key access object addressed to this agent's current TDF key, or the swarm's, and unwrap locally or through key release. Check the policy binding, decrypt and check the AES-GCM tag, then compare the inner `iss` and `nonce` and verify the carried statement against `stmt` and `iss`.
6. Hand the inner `rpc` to the standard A2A handler with a `TaskSecurityContext` built from the verified sender.

## Replies, streams and errors

- **A reply is an envelope from the responder,** addressed to the requester and sealed to the key its `stmt` names. It carries `re` and a JSON-RPC response with the request's `id`. A reply whose `re` matches no outstanding request is dropped.
- **Each stream event is its own envelope,** with `re` and a `seq` that rises from 0. A gap or a repeat is a stream error, never a normal end, so a lagging or truncated stream cannot pass as complete. The stream ends with A2A's terminal event. A push notification is an envelope addressed to the subscriber, posted to the URL it registered.
- **A refusal reveals nothing.** A document refused at any step from 1 to 5 gets one unsealed JSON-RPC error, with `id: null` (the real `id` is sealed), code `-32600` and the message `envelope refused`. Which check failed goes only to the local log, so a prober learns nothing from the answer. Once a request is admitted, A2A's errors, with its JSON-RPC codes unchanged, travel inside a sealed reply.
- **Pairing** follows INGRESS-012. Its body is the pairing exchange, a method extension rather than an A2A core operation, and any other body is refused after unwrap with no side effect.

## Authentication

The envelope signature is the binding's credential. An agent declares no A2A `securitySchemes` for this interface and sends no CWT to peers: the sender is verified on every message from its workload key, and its standing is decided at the receiver. A CWT minted from the workload key remains wherever a service outside the mesh verifies the agent, namely the network KAS and authnz-rs.

## TDF-JSON Part

```json
{
  "data": { "tdf": "json", "version": "1.0.0", "manifest": {}, "payload": {} },
  "mediaType": "application/tdf+json",
  "filename": "role-bundle.tdf.json"
}
```

- **It is an ordinary A2A 1.0 `Part`** whose `data` is a TDF-JSON document. It is not double-encoded, since `data` holds any JSON value.
- **It has its own key access and policy,** independent of the envelope's. A recipient outside that policy forwards the Part unopened. This is how an orchestrator or relay carries a role bundle or sealed knowledge it may not read.
- **Large content travels as a `url` Part** with the same `mediaType`, whose URL names the blob on the Iroh data plane. The blob is the TDF-JSON document. This replaces `AgentSpecializeRequest.ticket`.
- **A message that holds one lists the extension URI** in `Message.extensions`, and the sender activates it through the `a2a-extensions` service parameter. The extension's `params.formats` lists the TDF formats the agent opens.
- **`OpenTdfService` moves to `TdfJson`,** off the deprecated `TdfJsonRpc` (`crates/arkavo-tdf/src/opentdf_impl.rs`). `TdfJson` already requires the KAS public key that EC key wrapping needs.

## What the existing surfaces become

| Today | On the binding |
|---|---|
| `message/send` and the other JSON-RPC methods, in plaintext | The same operations under their A2A 1.0 names, sealed |
| `agent.specialize` with `encrypted_bundle` or `ticket` | `SendMessage` carrying a TDF-JSON Part, advertised as an AgentCard skill (DEC-4) |
| `tdf.share` and `tdf.offers` | `SendMessage` carrying a TDF-JSON `url` Part |
| `kas.rewrap` with caller `did:key:z6MkUnknown` | The caller is the verified envelope sender (INGRESS-016), until the KAS moves off the wire (WS5) |
| `gossip/message` checked against bound keys | Swarm-addressed envelopes from members (INGRESS-015) |
| CWT on calls between agents (realignment WS3) | Not sent; the envelope authenticates |

## Format negotiation

A client takes the first interface in `supportedInterfaces` that it supports, which is how A2A negotiates bindings. A TDF-CBOR binding (`https://arkavo.com/a2a/bindings/tdf-cbor/v1`, over opentdf-rs's `tdf_cbor`) would be listed ahead of TDF-JSON once both sides support it. TDF-JSON stays listed as the fallback (TDFS-005). The Part extension's `params.formats` plays the same role for Parts. NanoTDF is not used for messages.

## Profile deviations from OpenTDF

Stock OpenTDF SDKs are not the audience for agent messages, but the deviations are explicit:

- **A key access object's `url` holds a `did:key`** for an agent-addressed document. An SDK would try it as a KAS URL. Its `kid` is a statement digest, not a KAS key id.
- **The envelope assertion's JWS signs a digest of the whole canonical document** rather than OpenTDF's `assertionHash` and `assertionSig` claims, with ES256 from the sender's workload key.
- **opentdf-rs 0.16.1 parses assertions but does not sign or verify their bindings.** The seal/open crate implements the `jws` binding, and the change goes upstream to opentdf-rs.

## Cost

Sealing costs one ECDH, one AES-GCM pass and one ES256 signature. Opening costs one ES256 verification, one ECDH and one AES-GCM pass. A Secure Enclave signature takes a few milliseconds (`crates/arkavo-server/src/server/well_known.rs`), and every stream event is signed, so the seal/open crate benchmarks both directions against the 50 ms routing budget, with enclave signing measured separately. A plaintext carrier still exposes traffic patterns and lets an attacker drop, delay or reorder documents. That denial of service is accepted, as in the trust model.

## Decisions

- **The envelope authenticates agent-to-agent traffic; no CWT on the binding.** CWT stays for services outside the mesh.
- **The workload key signs envelopes with ES256.** This replaces the trust model's COSE_Sign1 EdDSA outer layer, which was written when the workload key was Ed25519 and when opentdf-rs 0.15 could not decode assertions. With #766 the workload key is P-256, and `TdfJson` in 0.16.1 parses `manifest.assertions`, so one TDF-JSON document carries both the seal and the signature.
- **The identifier stays on arkavo.com, and the binding is offered to A2A.** This document covers what A2A asks a binding specification to state: data mapping, service parameters, error mapping, streaming and authentication. It is proposed to a2aproject as an experimental custom binding (`experimental-cpb-tdf-json`). If the project assigns an identifier, cards list both interfaces until peers have moved.
- **TDF-JSON for messages and Parts, not NanoTDF.** This resolves the trust model's open "message format" decision.

## Delivery

The stack below this layer delivers the workload key, key statements, the signed card carrying them and the kept TDF key. The remaining layers, each a layer of the same stack:

- **Seal and open.** A crate taking bytes in and returning the verified sender and the inner JSON-RPC, with no A2A dependency, over `opentdf::jsonrpc::TdfJson`, `arkavo-key-statement` and `arkavo-crypto`. It has a test for every refusal step (INGRESS-009, INGRESS-010) and for replies and streams (INGRESS-011), plus a benchmark.
- **Statement digest in the TXT record and QR code** (INGRESS-007, INGRESS-012).
- **Ingress.** The agent listener accepts the binding, public JSON-RPC shrinks to public requests (INGRESS-002), and the KAS caller becomes the verified sender (INGRESS-016).
- **Gossip as swarm-addressed envelopes** (INGRESS-015).
- **TDF-JSON Part.** `OpenTdfService` moves to `TdfJson`, and specialization and `tdf.share` become `SendMessage` with a Part (TDFS-018).
- **Realignment.** `arkavo-a2a-edge` lists and serves the binding (WS2), the 1.0 card shape lands (INGRESS-018), Swift implements seal and open, and the conformance matrix gains a binding row (WS10, WS11).
- **Upstream.** `jws` assertion bindings go to opentdf-rs, and the binding goes to a2aproject.
