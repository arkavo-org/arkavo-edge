# Arkavo Behavior Specifications

Machine-readable BDD specifications for the Arkavo Edge platform.

## Structure

```
specs/
├── schema.json              # JSON Schema for validation
├── README.md                # This file
└── arkavo-edge/             # Per-component specs
    ├── registration.spec.yaml
    ├── crypto.spec.yaml
    ├── chat-session.spec.yaml
    └── gossip-protocol.spec.yaml
```

## Specification Format

Each `.spec.yaml` file follows the schema defined in `schema.json`:

```yaml
feature: Human-readable feature name
module: rust::module::path
version: x.y.z

invariants:
  - Global rules that must always hold

scenarios:
  - id: UNIQUE-001           # Short identifier
    name: Descriptive name
    criticality: high        # critical | high | medium | low
    given: [preconditions]
    when: action/trigger
    then: [expected outcomes]
    refs: [source:lines]     # Code references
```

## Usage

### For AI Agents

```rust
#[derive(Deserialize)]
struct BehaviorSpec {
    feature: String,
    module: String,
    scenarios: Vec<Scenario>,
}

// Load specs for context
let spec: BehaviorSpec = serde_yaml::from_str(include_str!(
    "../../specs/arkavo-edge/registration.spec.yaml"
))?;
```

### For PR Reviews

When behavior changes, the spec diff shows intent:

```diff
  - id: REG-003
-   name: Accept any valid key
+   name: Verify challenge with P-256 signature
+   changed: 2025-01-30
    given:
+     - P-256 SEC1 key from iOS Secure Enclave
    when: verify_challenge called
    then:
-     - Signature verified
+     - Detects key type by length
+     - Verifies ECDSA signature with SHA-256
```

### Validation

```bash
# Validate all specs against schema
npx ajv-cli validate -s specs/schema.json -d "specs/**/*.spec.yaml"
```

## Contributing

1. Add new scenarios with sequential IDs within the component
2. Update `changed:` date when modifying existing scenarios
3. Include `refs:` pointing to relevant source code
4. Run validation before committing
