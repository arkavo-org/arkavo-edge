# TAP local runbook

Everything here is development-only: the registry is a local trust authority, the grant is a
`local-test-grant`, and payment is simulated. Nothing is a Visa registration, an AP2 mandate or
a payment.

## Acceptance suite

```bash
cargo run -p arkavo-tap-demo -- verify-suite
```

It starts a registry and a merchant in-process, in a fresh temporary directory, with an
injectable clock and independent clients, so it can test clock edges, key revocation and a
registry outage. It never reads or changes `.tap-demo`, so it takes no `--config`. Each line
reports one scenario from [specification.md](specification.md):

- `PASS` / `FAIL`: run end to end.
- `UNIT`: needs a lower-level seam than HTTP. Covered by the named `arkavo-tap` tests: T11
  (two verifiers sharing a replay store), T12 (replay after restart), T25 (RFC 9421 vectors and
  `httpsig` in both directions).
- `OFF`: the capability is disabled. T16 (consumer recognition: no canonical serialization
  upstream) and T27 (AP2 not integrated).

`cargo test -p arkavo-tap-demo` runs the same suite and fails if any scenario fails.

## Manual walk-through

```bash
# Separate identities (agent A, agent B, merchant, consent issuer), configuration and a
# bounded grant for agent A. Key files are mode 600. Refuses to overwrite anything.
cargo run -p arkavo-tap-demo -- init --dir .tap-demo

# Terminal A: the development registry on 127.0.0.1:9002.
cargo run -p arkavo-tap-demo -- registry --config .tap-demo/config.toml

# Terminal B: the merchant fixture with TAP verification on 127.0.0.1:3001.
cargo run -p arkavo-tap-demo -- merchant --config .tap-demo/config.toml

# Terminal C: register the agents' public keys (loopback-only control plane).
cargo run -p arkavo-tap-demo -- register --config .tap-demo/config.toml

# Browse as a verified test agent: shows the verified actor and trust authority, and that no
# principal is resolved and no purchase authority is implied.
cargo run -p arkavo-tap-demo -- browse --config .tap-demo/config.toml --product demo-001

# Check out and commit a simulated order under the signed test grant. Prints the checkout,
# the authorization result, the simulated payment state and the merchant-signed receipt.
cargo run -p arkavo-tap-demo -- checkout --config .tap-demo/config.toml --grant .tap-demo/approval.signed.json

# An order's status, visible only to the agent that placed it.
cargo run -p arkavo-tap-demo -- order --config .tap-demo/config.toml --id <order id>
```

The merchant logs one `arkavo_tap::audit` record per protected request, accepted or rejected,
with its decision ID; responses carry the same ID in `x-correlation-id`. Refusals return only
an error code and that ID.

### Fixture contract

| Endpoint | Protection |
| --- | --- |
| `GET /health` (registry and merchant) | None; makes no identity claim |
| `POST /dev/v1/agents`, `POST /dev/v1/agents/{key_id}/revoke` | Registry control plane, loopback only |
| `GET /dev/v1/keys/{key_id}` | Registry key documents, cached by verifiers for `key_max_age_seconds` |
| `GET /demo/v1/catalog` | Anonymous; never verified-agent status |
| `GET /demo/v1/products/{id}` | Read, `agent-browser-auth` |
| `POST /demo/v1/checkouts` | Checkout coverage, `agent-payer-auth`; server-computed, immutable checkout |
| `POST /demo/v1/orders` | Commit coverage, `agent-payer-auth`, plus a `local-test-grant`; idempotent |
| `GET /demo/v1/orders/{id}` | Read; only the placing agent |

Every method on a protected path is protected (a `HEAD` to a `GET` route must verify too).

### Revocation policy

A verifier uses a key record until its refresh deadline (`maxAgeSeconds` in the key
document; `key_max_age_seconds = 60` in the generated configuration). A revocation therefore
takes effect within that interval. Past the deadline, a registry that cannot be reached makes
protected requests fail closed with 503; a stale record is never revived.

## Arkavo Market: staging notes

Not performed. No signed request has been sent to market.arkavo.com or to any staging host.
What a staging integration needs, from the market as deployed today:

- **Endpoints.** The market serves pre-rendered pages (`/`, `/@<handle>/<slug>`, `/llms.txt`,
  sitemaps) from `market-serve`, which terminates TLS. `market-commerce`, on a local socket,
  answers `/claim/<checkoutSessionId>` and `/.well-known/jwks.json`. Checkout itself is a Stripe
  Payment Link, so the market has no checkout or order API of its own for TAP to protect yet.
- **Placement.** Verify in-process in `market-serve`, before routing, with audience
  `https://market.arkavo.com`; it already terminates TLS, so no forwarded context is trusted. The
  verifier needs a registry resolver and a durable replay store, and no secrets.
- **Trust.** A staging trust authority with its own environment name, never `local`, and none of
  this harness's keys or simulated payment.
- **Purchases.** A signed Stripe Checkout redirect does not bind the payment to the agent's
  evidence. Committing an order on an agent's behalf needs a merchant-side checkout reference
  that the authorization decision can bind, then AP2 or an equivalent purchase grant.
