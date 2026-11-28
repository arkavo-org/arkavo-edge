# OpenTDF and Keycloak agent demo

**Date:** 2026-10-18

**Status:** Proposed design for a demo. Nothing here is built. Tracked in arkavo-edge#771, with the work in #767 (DPoP and signed rewraps), #768 (Keycloak agent tokens), #769 (owner console) and #770 (encrypt receipts), and in arkavo-org/opentdf-platform#56 and #57.

**Scope:** arkavo-edge agents, the upstream OpenTDF platform, Keycloak, and an owner console. It relates to `docs/swarm-trust-model.md`, arkavo-edge#766 (the workload key in the Secure Enclave), arkavo-edge#735 (the agent KAS caller), arkavo-edge#711 (trust states) and arkavo-org/authnz-rs#102 (agent contract v3).

**Code read:**
- `P` = opentdf/platform at [`b048476`](https://github.com/opentdf/platform/tree/b04847659c0fe82a3b2cf47ca61f8dbba33218e6) (service v0.29.0, sdk v0.35.0)
- `F` = arkavo-org/opentdf-platform at [`e562aca`](https://github.com/arkavo-org/opentdf-platform/tree/e562aca08d48f99c705c335909b7265e8c6fed00)
- `K` = Keycloak at tag [`26.8.1`](https://github.com/keycloak/keycloak/tree/24ed1dadf3f246a555adb6d3d4a99e9b3003fb3b)
- `A` = authnz-rs at [`6ff7799`](https://github.com/arkavo-org/authnz-rs/tree/6ff7799125f6f644c8556d284990581e7b96817e)

## Goal

A human registers an AI agent and decides what it may read. The human sees every TDF the agent creates and opens, and can restrict, quarantine or retire it, with each action taking effect at the next key request.

The demo shows agent identity governed like an employee's:
- the owner authorizes the agent once;
- the agent proves its key for every short-lived token;
- the owner re-certifies it every 90 days;
- a Guardian or the owner can stop it at any time.

The demo uses upstream components so it runs without Arkavo infrastructure. It also tests whether the identity decisions in arkavo-edge#766 and authnz-rs#102 hold up against a mainstream identity provider.

## Decision

Run the upstream OpenTDF platform (service v0.29.0) with stock Keycloak 26.8.1. Each agent is its own confidential Keycloak client:
- It authenticates with `private_key_jwt` (ES256) using its P-256 workload key.
- Its tokens are DPoP-bound to that same key.
- A small owner console drives the Keycloak admin API and the OpenTDF policy API, and reads the platform's audit log.

The alternatives were rejected because each is blocked today:

- **The Arkavo fork with authnz-rs** is the production path, but it cannot show human management yet.
  - authnz-rs v2 accepts only Ed25519 agent keys (`A/src/agent.rs#L229-L266`).
  - The fork demands a DPoP proof for every agent CWT, because each carries `cnf` (`F/service/internal/auth/authn.go#L536-L550`). Edge never sends one: opentdf-rs 0.16.1 supports `with_caller_key`, and nothing in `crates/` calls it.
  - No owner-facing console exists.
- **Federation** fails in both directions.
  - The fork accepts CWTs from exactly one issuer and refuses to start without `cose_keys_uri` (`F/service/internal/auth/token_verifier.go#L11-L79`), so it cannot also trust Keycloak.
  - Keycloak cannot broker humans through authnz-rs, because `/oauth/authorize` has no browser passkey login (`A/src/oidc.rs#L2016-L2069`), and `agents:delegate` refuses Google sign-in.

The workload key and its `did:key` are the same on both paths. Choosing Keycloak for the demo therefore changes no agent-identity decision, and the agent can later register with authnz-rs v3 unchanged.

## Demo script

Alice is a human in the `arkavo-demo` realm. She signs in to the owner console with a passkey. Keycloak supports passkeys for users (`K/docs/documentation/server_admin/topics/authentication/passkeys.adoc`), and the console needs no password.

- **Register.** Alice starts `arkavo agent --trust` on her Mac. The agent shows its pairing QR code, which carries the workload `did:key:zDn…` and a single-use secret. She scans it with the console.
  - The console challenges the agent to sign a nonce with the workload key, then creates the agent's Keycloak client.
  - Alice picks entitlements from a list limited to what she holds herself, for example `project/alpha` but not `project/bravo`.
  - She never types an identifier.
- **Work.** The agent mints a token, encrypts a summary under `project/alpha` plus its own `origin` value, and decrypts two `project/alpha` documents.
  - The console's activity view shows one encrypt receipt and two permitted rewraps. Each names the agent, the policy UUID, the attribute values and the KAS key.
- **Probe.** The agent tries a `project/bravo` document. The console shows a denied rewrap, with the decision event explaining that no mapping grants `bravo`.
- **What can it read.** The console shows the agent's entitlements: the same answer as `otdfctl policy subject-mappings match` for the agent's token.
- **Restrict.** Alice removes `project/alpha` from the agent. Its next decrypt is denied, while its unexpired token is still in hand.
- **Contain its output.** Alice withdraws read on the agent's `origin` value. Nobody can open what the agent wrote any more, while every other `project/alpha` document stays readable. Restoring the mapping restores access.
- **Quarantine.** A Guardian disables the agent.
  - The next rewrap is denied, and the token endpoint refuses a new token.
  - Only Alice can recover it, and recovery is recorded.
- **Review.** With a demo review interval of minutes instead of 90 days, the agent becomes suspended when its review falls due. Alice re-certifies it with a passkey tap and it resumes.
- **New key, new agent.** Deleting the agent's key file and restarting gives a new DID. The old client stays bound to the old key, and the new agent starts unregistered.

## Architecture

| Component | Role | Version |
|---|---|---|
| Keycloak | Issues every token; holds agent clients and their owner metadata | 26.8.1 (supported DPoP since 26.4, workflows since 26.6) |
| OpenTDF platform | Policy, authorization v2, Keycloak entity resolution (ERS) and KAS in one process | service v0.29.0 |
| Postgres | Platform and Keycloak storage | as in `P/docker-compose.yaml` |
| Owner console | Registers, entitles, reviews, quarantines and shows activity | new |
| arkavo-edge agent | Holds the workload key; mints tokens; encrypts and decrypts with opentdf-rs; writes encrypt receipts | Edge with #766 step 1 |

| Token | Issued to | Grant | Bound to |
|---|---|---|---|
| Owner session | Alice in the console | Authorization code with PKCE, WebAuthn | console session |
| Agent access token | The agent's own client | `client_credentials` with `client-jwt` assertion | DPoP `cnf.jkt` = the workload key |
| Console service token | The console's confidential client | `client_credentials` with `client-jwt` | the console's own key; holds `manage-clients` |
| ERS token | `tdf-entity-resolution` client | `client_credentials` | not bound; holds `view-clients`, `view-users` |

**Agent tokens can do almost nothing but rewrap, so the KAS is where control has to bite.**
- Every token gets the casbin role `unknown`, which allows only `Rewrap` (`P/service/internal/auth/authz/casbin/v1/casbin_policy.csv#L16-L34`).
- Agents are given no `opentdf-standard` role.
- Fetching a KAS public key to encrypt is a public route (`P/service/internal/auth/authn.go#L41-L51`).

**Platform configuration departs from `opentdf-dev.yaml` in these ways:**
- `server.auth.dpop.enforce: true`, so no bearer-only token reaches the KAS.
- `audit.jwt_claim_mappings` copies `azp` into rewrap events.
- Stock Keycloak has no `/auth` prefix, so `legacykeycloak` is off.
- The entitlement policy cache stays off, so policy edits apply at the next decision.

## How an agent is registered

**The console creates one confidential client per agent through the admin REST API.** It acts only after verifying the owner's session and the agent's proof of possession.

**Client settings:**
- `clientAuthenticatorType: client-jwt` and `token.endpoint.auth.signing.alg: ES256`.
- `jwks.string` holds the workload key as an EC JWK with `"use":"sig"`. Keycloak silently ignores an inline key that lacks `use` (`K/services/src/main/java/org/keycloak/keys/loader/ClientPublicKeyLoader.java#L74-L81`).
- `kid` is the `did:key` string, so the identity shows wherever the key does.
- `dpop.bound.access.tokens: true` and `access.token.lifespan: 300`.
- `serviceAccountsEnabled: true`; standard flow, direct grants and token exchange off.
- Attributes:
  - `arkavo_agent=true`
  - `arkavo_did=<did:key>`
  - `arkavo_owner=<owner user id>`
  - `arkavo_state`
  - `arkavo_review_due`

**The clientId is `agent-` followed by the lowercase hex of the key's RFC 7638 thumbprint.** It cannot be the `did:key` string, for two reasons:
- Keycloak lowercases every username, service accounts included (`K/model/jpa/src/main/java/org/keycloak/models/jpa/JpaUserProvider.java#L133`).
- The ERS finds the client again by stripping `service-account-` and looking the remainder up by exact match (`P/service/entityresolution/keycloak/v2/entity_resolution.go#L429-L448`, `K/services/src/main/java/org/keycloak/services/resources/admin/ClientsResource.java#L166`).

A base58 `zDn…` clientId would come back lowercased and miss. The ERS would then fall back to a user entity with no `clientId`, and every agent mapping would silently stop matching.

The hex thumbprint also keeps "the account is the key": a new key derives a new clientId, and the token's `cnf.jkt` is the same thumbprint in base64url.

**A client policy holds every agent client to this shape.** The condition is `client-attributes` with `arkavo_agent=true`. The profile runs:
- `secure-client-authenticator`, allowing only `client-jwt`;
- `secure-signature-algorithm-signed-jwt`;
- `dpop-bind-enforcer`.

This keeps a later admin edit from quietly turning an agent into a secret-holding client (`K/docs/documentation/server_admin/topics/clients/client-policies.adoc#L60-L108`).

**Owners do not get Keycloak admin rights over their agents.**
- Fine-grained admin permissions v2 can grant `manage` on one client. That right would also let the owner replace the client's key and edit `arkavo_owner`, which would break both the key invariant and the entitlement model.
- The console holds `manage-clients` instead and exposes only the operations below.
- Keycloak admin events record every change it makes, once they are enabled in the realm's event settings (they are off by default; `K/docs/documentation/server_admin/topics/events/admin.adoc`).

**Entitlements live in OpenTDF, not in Keycloak.** The console writes one subject condition set per agent:

```text
.clientId IN ["agent-<hex>"]  AND  .enabled IN_CONTAINS ["true"]
```

It adds one subject mapping per granted attribute value, with actions `read` and `create`.

- **`.clientId` makes a value reachable only by that agent.** Only Keycloak client representations carry `clientId`; user representations do not. The upstream fixtures use the same selector for `opentdf-sdk` (`P/service/internal/fixtures/policy_fixtures.yaml#L347-L357`).
- **`.enabled` makes quarantine immediate at the KAS.**
  - The ERS fetches the live client on every decision, because its cache is off by default (`P/service/entityresolution/v2/entity_resolution.go#L50-L66`).
  - `IN_CONTAINS` stringifies the boolean (`P/service/internal/subjectmappingbuiltin/subject_mapping_builtin.go#L245-L259`), so a disabled client stops matching.
  - This is read from code, not run.

**The owner's ceiling is checked by the console when it writes a mapping.**
- The console asks authorization v2 for Alice's entitlements and refuses values she lacks.
- The check is not live: if Alice later loses `project/alpha`, her agent keeps it until the next review or until the console reconciles.
- The fork path is stronger here. authnz-rs computes `arkavo_entitlements` as the delegated set intersected with the owner's current set each time it mints a token (`A/docs/agent-credentials-contract.md#L43-L65`).

## How the key is bound

One P-256 workload key signs three things on this path. Their required claims do not overlap (`iss`/`sub`/`aud` for the assertion; `typ: dpop+jwt`, `jwk`, `htm`, `htu` for the proof; `requestBody` for the SRT), so a verifier of one should not accept another. That is inferred from the verifiers, not tested:

| Signature | Verifier | What it proves |
|---|---|---|
| Client assertion (`iss` = `sub` = clientId, `aud` = token endpoint, `jti`, `exp` ≤ 60 s) | Keycloak `client-jwt` | The caller holds the registered key |
| DPoP proof at the token endpoint and at the KAS | Keycloak, then the platform (`P/service/internal/auth/authn.go#L1277-L1431`) | The token is bound to the key (`cnf.jkt`) and is used by its holder |
| Signed request token (SRT) on rewrap | KAS (`P/service/kas/access/rewrap.go#L423-L449`) | The ephemeral `clientPublicKey` the DEK is wrapped to was chosen by the key holder |

**The SRT check depends on DPoP.**
- KAS verifies the SRT signature only when the request carried a DPoP key. Without one, it checks only `iat` and `exp`.
- A bearer token without DPoP therefore lets whoever holds it choose the key the DEK is wrapped to.
- DPoP-bound clients and `dpop.enforce` close that.

**The rewrap session key is ephemeral software, not the enclave key.**
- The DEK comes back wrapped to an ephemeral RSA or EC key the client generates per session.
- That keeps the workload key out of encryption, as "One workload key" in the trust model requires.
- opentdf-rs uses an ephemeral RSA key for a ZTDF rewrap ([opentdf-rs `src/kas.rs#L459`](https://github.com/arkavo-org/opentdf-rs/blob/7cdc9451a216525d62c4f01443410749fe44476d/src/kas.rs#L459)), so the platform's `preview.ec_tdf_enabled` stays off. It is needed only for EC-wrapped responses (`P/service/kas/access/rewrap.go#L1225-L1238`).

**Enclave signing reaches these protocols in two steps.**
- **Software P-256 first.** opentdf-rs's `CallerKey::P256` takes a software `p256::ecdsa::SigningKey` ([opentdf-rs `src/kas_dpop.rs#L36-L41`](https://github.com/arkavo-org/opentdf-rs/blob/7cdc9451a216525d62c4f01443410749fe44476d/src/kas_dpop.rs#L36-L41)). This matches the software tier #766 already plans for hosts without hardware, so the whole demo runs on it first.
- **Secure Enclave second.** A non-exportable enclave key needs a signer variant in opentdf-rs: a callback or trait object that returns a 64-byte `r‖s` signature. The Go SDK has the same limitation, since it requires a raw private key (`P/sdk/dpop_key.go#L168-L191`).
  - Each enclave signature takes about 4.5 ms (authnz-rs#102), so a rewrap costs about 9 ms of signing.

**Keycloak does not require the DPoP key to be the client's registered key.** Only the key holder can obtain a token, but it could bind that token to a different key. Edge always uses the workload key. Because the clientId embeds the thumbprint, the platform could check this from the token alone; upstream does not.

## How agent TDFs become visible

**The platform records decrypts and policy changes, but not encrypts.**

| Event | Recorded by | Fields used |
|---|---|---|
| Rewrap (one per key access object) | KAS audit, `object.type` `key_object` | `object.id` (the policy UUID), attribute FQNs, `keyID`, `algorithm`, result, `requestID`, `azp` from the claim mapping (`P/service/logger/audit/rewrap.go#L57-L128`) |
| Decision | Authorization v2, one event per entity | `actor.id` `jwtentity-N-clientid-agent-<hex>`, entitlements, per-resource decision, obligations (`P/service/internal/access/v2/just_in_time_pdp.go#L656-L688`) |
| Policy change | Policy services | actor `sub`, the original and updated object (`P/service/logger/audit/policy.go#L30-L112`) |
| Encrypt | nothing upstream | — |

**Encrypts become visible through agent receipts.**
- After each encrypt, the agent sends the console a receipt signed by its workload key. It holds:
  - the manifest digest;
  - the policy UUID and attribute FQNs;
  - the KAS URL and key id;
  - the time.
- A receipt is the agent's own claim, so it reaches the honest-worker tier only.
- The console treats the platform as the authority. A policy UUID that shows up in rewraps with no receipt is flagged as an unreported creation.

**The console joins the two platform events on `requestID`.**
- The rewrap event's `actor.id` is the service account's user UUID (`P/service/logger/audit/utils.go#L196-L215`), and its `actor.attributes` are always empty.
- The `azp` claim mapping and the decision event both name the agent client.
- The audit log goes only to stdout as JSON (`P/service/logger/logger.go#L93-L106`), so the demo ships container logs to the console.

**Each owner control is a policy edit the KAS sees on the next request:**
- **Restrict:** delete the agent's mapping for a value.
- **Contain an agent's output:**
  - Every agent has an `origin` attribute value (`https://demo.arkavo.net/attr/origin/value/agent-<hex>`), which Edge adds to every TDF it creates.
  - The console maps that value to every enabled user and client with `.enabled IN_CONTAINS ["true"]`. Both Keycloak user and client representations carry `enabled`.
  - A decision must satisfy every attribute definition on the TDF (`P/service/internal/access/v2/evaluate.go#L205-L227`). So `origin` never widens access beyond the TDF's other values; it only lets the owner withdraw it.
  - Narrowing or deleting the `origin` mapping stops reads of everything that agent wrote, such as after a quarantine, without touching anyone's `project/alpha` access.
- **Quarantine:** disable the client. New tokens fail with `CLIENT_DISABLED`, and the `.enabled` condition denies the next rewrap.
- **Retire:** delete the mappings and the client. A TDF stays readable by anyone else entitled to its values.

**Some things these controls cannot undo.**
- Plaintext or keys already released stay with the agent.
- KAS releases a TDF whose policy has no data attributes without asking for a decision (`P/service/kas/access/accessPdp.go#L71-L101`), so the console marks any attribute-free TDF in a receipt as uncontrolled.

**Registered resources and obligations do not help this demo.**
- A registered resource value can stand in as an entity, but nothing binds it to the caller (`P/service/internal/access/v2/pdp.go#L436-L543`).
- Obligation fulfilment is self-reported by the caller (`P/service/internal/access/v2/obligations/obligations_pdp.go#L165-L216`).
- Both are useful later as hints to Edge's own gate, not as controls the owner relies on.

## Review and quarantine

**Keycloak has two client states, enabled and disabled.** The trust states from #711 are kept in `arkavo_state` and enforced by the console:

| #711 state | Keycloak | Set by |
|---|---|---|
| Eligible | enabled | registration, re-certification, recovery |
| Suspended | disabled, `arkavo_state=suspended` | the console's review job once `arkavo_review_due` passes |
| Quarantined | disabled, `arkavo_state=quarantined` | Guardian or owner through the console |
| Unassessed | not applicable | a client exists only after the owner authorizes it |

**The 90-day review is the console's job.**
- Keycloak has no access-review or client-expiry feature.
- Its workflows can disable a client after a period without `client-authenticated` events (`K/docs/documentation/server_admin/topics/workflows/defining-steps.adoc#L103-L112`, `K/docs/documentation/server_admin/topics/workflows/listening-workflow-events.adoc#L55-L56`). That is an inactivity rule, not a review, and the demo uses it as a second, independent stop.
- Re-certification requires a fresh WebAuthn assertion from the owner. It moves `arkavo_review_due` and re-enables the client.

**Keycloak cannot hold a quarantine latch.** Any admin with `manage` on the client can re-enable it. The console refuses recovery except by the owner, and Keycloak admin events show anyone who went around it. The fork path holds the latch in the issuer (`A/docs/agent-credentials-contract.md#L163-L197`).

**Timing is bounded by the next KAS decision, not by token lifetime.**
- The `.enabled` condition and policy edits apply when the ERS next fetches the client.
- Keycloak cannot revoke JWTs already issued. The platform validates them locally, and RFC 7009 revocation needs the agent's own credentials (`K/services/src/main/java/org/keycloak/protocol/oidc/endpoints/TokenRevocationEndpoint.java#L232-L246`).
- Because the agent's token can do nothing but rewrap, a 300-second token outliving a quarantine gains nothing.
- This does not meet the trust model's 1-second propagation target under load, and nothing here measures it.

## Upstream today versus what must be built

| Capability | Upstream today | Must build |
|---|---|---|
| Agent as its own client with a P-256 key and no secret | Keycloak `client-jwt` and ES256 | Edge token client: OIDC discovery, assertion, DPoP |
| Token bound to the agent key | Keycloak DPoP; platform DPoP with ES256 | Edge sends DPoP and SRT through opentdf-rs `with_caller_key` |
| Secure Enclave signing | — | #766 step 1; a signer variant in opentdf-rs |
| Agent-only entitlements | Subject mappings on `.clientId` | Console writes them under the owner's ceiling |
| Immediate quarantine at the KAS | `.enabled` condition (read from code, not run) | Console sets it; demo verifies it |
| 90-day review | — | Console review job and re-certification |
| Decrypt visibility | KAS and decision audit events | Log shipping and the console's join |
| Encrypt visibility | — | Signed agent receipts |
| Person and agent decided together | — | Not in this demo |

**Person and agent are not decided together.**
- Keycloak 26.8 can issue delegated tokens carrying `act` as a preview feature (`K/common/src/main/java/org/keycloak/common/Profile.java#L94`), but the Keycloak ERS reads only `azp` and `preferred_username`.
- It files the acting agent as `azp`, an ENVIRONMENT entity, and the PDP drops ENVIRONMENT entities (`P/service/internal/access/v2/just_in_time_pdp.go#L546-L558`).
- A delegated token would therefore give the agent all of its human's rights.
- An ERS that turned `act` into a second SUBJECT would make the PDP intersect the two, since it requires every SUBJECT to be entitled (`P/service/internal/access/v2/just_in_time_pdp.go#L256-L300`). That is upstream work for later; the demo keeps the agent as its own subject.

## Risks

- **Two identity paths.** Keycloak is not the production issuer. The workload key, its DID and the trust states must stay issuer-neutral in Edge, or the demo forks the agent's identity code.
- **Silent mismatches.**
  - A clientId that is not lowercase, or a JWKS key without `use`, fails without an error. Edge derives the clientId and the console builds the JWK, so neither is typed by hand.
  - The ERS also fails if `preferred_username` is missing from the token (`P/service/entityresolution/keycloak/v2/entity_resolution.go#L419-L427`). That claim comes from the default `profile` scope, so agent clients keep the realm default scopes, and the console never turns on lightweight access tokens for them.
- **Optional DPoP.** Without the client policy and `dpop.enforce`, one misconfigured agent client falls back to bearer tokens and an unverified SRT.
- **Console privilege.** The console holds `manage-clients` and policy write rights. Casbin v2 can limit it to the policy RPCs it uses (`P/service/internal/auth/authz/casbin/v2/policy.csv#L1-L59`).
- **The owner's ceiling drifts.** It is checked when a mapping is written, not when a key is released.
- **Receipts and `origin` are agent-asserted.** A compromised agent can skip receipts or leave `origin` off a TDF, so both reach the honest-worker tier only. Only rewraps are authoritative.
- **Version drift.**
  - Upstream tests Keycloak 26.4.0 (`P/docker-compose.yaml#L11`), and the demo runs 26.8.1.
  - Registered resources and obligations are still moving upstream.
  - Keycloak has no DPoP nonce yet ([keycloak#39042](https://github.com/keycloak/keycloak/issues/39042)).

## Impact on the identity work in progress

**arkavo-edge#766, the workload key in the Secure Enclave.** The demo depends on step 1 and adds requirements to it:
- The enclave shim exposes one ES256 signing call that returns `r‖s`. JOSE callers (client assertion, DPoP, SRT, receipts) each sign their own typed structure through it, and the authnz-rs challenge goes through it only with domain separation (see #102 below).
- It exports the public key as an EC JWK, with `kid` set to the DID.
- Edge derives the lowercase thumbprint id for systems that fold case.
- The software P-256 tier needs to work first, because the demo starts there.
- Step 2 ("register the enclave key with the identity provider") gains a Keycloak adapter next to the authnz-rs one.

**authnz-rs#102, contract v3.** The demo does not need it, because Keycloak authenticates P-256 clients today; production still does. Three changes follow:

- **Remove a stale sentence.** The fork already accepts an EC2 P-256 `cnf` and EdDSA DPoP, since fork PR #49. The statement that a P-256 `cnf` gets past "the KAS's current ES/RS/PS-only DPoP check" is out of date.
- **Domain-separate the challenge.**
  - Today's challenge signature is ECDSA over whatever bytes the server sends, and Edge signs them without checking (`crates/arkavo-agent-auth/src/client.rs#L145-L149`).
  - Once the same key signs JWS for Keycloak and the KAS, a hostile or impersonated identity server could send a JWS signing input as the "challenge" and receive a valid DPoP proof or SRT signature.
  - v3 should sign a structured, typed payload (for example a COSE_Sign1 with a fixed content type over `{aud, nonce, challenge}`) or a fixed-prefix message. Edge should refuse any challenge that is not 32 bytes.
  - The same exposure arrives sooner on the Ed25519 key. The fork accepts EdDSA DPoP, so once Edge signs DPoP proofs with today's agent key, a raw-challenge signature with that key is an EdDSA JWS signature too. The 32-byte check has to land no later than Edge's first DPoP proof.
- **Sequence the identifier change with the fork.**
  - The fork's `cnf` check accepts only an absent `alg`, -7 or -8 (`F/service/internal/auth/cose_cnf.go#L136-L146`).
  - Its CWT verifier accepts only ES256 (`F/service/internal/auth/cwt_verifier.go#L124-L127`).
  - Until the fork accepts both identifiers, v3 should omit `alg` on `cnf`, which it does today, and keep the token's `alg` at ES256.

**The P-256 decision gains support.** Upstream DPoP accepts no EdDSA (`P/service/internal/auth/authn.go#L56-L67`), so an Ed25519 agent could not bind its tokens on stock OpenTDF.

**The tenure model maps onto `arkavo_review_due` and the console's job.** It should keep #102's semantics: suspended rather than revoked, and re-certified without a new state version.

**arkavo-edge#735, the agent KAS caller.** Keycloak does not replace it, because the agent KAS serves peer shares without the network KAS. Two changes would let the console show both KASes:
- Record its decisions with the same fields as the platform's rewrap event: policy UUID, attribute FQNs, key id and result, plus the caller DID.
- Accept a P-256 sender.

The console maps the DID to the Keycloak client through `arkavo_did`.

**The Edge KAS client blocks both paths today.**
- It posts to a fixed `<oauth_url>/oauth/token` with a client secret (`crates/arkavo-tdf/src/kas_client.rs#L185-L196`).
- It sends no DPoP proof.
- The fork rejects its agent CWTs for lack of a proof, and Keycloak needs discovery and `client-jwt`.

Fixing DPoP unblocks the production path whether or not the demo goes ahead.

**The fork's owner-id client id remains.**
- The fork's Arkavo configuration sets `client_id_claim: arkavo_account_id` (`F/examples/config/opentdf-arkavo-local-test.yaml#L20-L22`).
- The authorization request context, and obligation triggers keyed on a PEP client id, therefore see an agent as its owner.
- The ERS code that copied the id was removed in fork #55. The configuration is what is left.
