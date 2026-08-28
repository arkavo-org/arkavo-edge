# Arkavo Behavior Specifications

Machine-readable BDD specifications for the Arkavo Edge platform.

## Structure

```
specs/
├── schema.json                 # JSON Schema for validation
├── README.md                   # This file
├── FUTURE.md                   # Roadmap and future work
└── arkavo-edge/                # Per-component specs
    ├── index.yaml              # Component index
    ├── registration.spec.yaml  # 12 scenarios
    ├── crypto.spec.yaml        # 11 scenarios
    ├── chat-session.spec.yaml  # 13 scenarios
    ├── gossip-protocol.spec.yaml # 8 scenarios
    ├── router.spec.yaml        # 10 scenarios
    ├── tdf.spec.yaml           # 9 scenarios
    ├── autolearn.spec.yaml     # 8 scenarios
    ├── memory.spec.yaml        # 7 scenarios
    ├── budget.spec.yaml        # 7 scenarios
    ├── authorization.spec.yaml # 6 scenarios
    ├── device-identity.spec.yaml # 6 scenarios
    ├── observability.spec.yaml # 8 scenarios
    ├── mcp-tools.spec.yaml     # 10 scenarios
    ├── events.spec.yaml        # 8 scenarios
    ├── dataflow.spec.yaml      # 6 scenarios
    ├── task-orchestration.spec.yaml # 8 scenarios
    ├── agent-auth.spec.yaml    # 6 scenarios
    ├── agent-cwt.spec.yaml     # 3 scenarios
    ├── llm-core.spec.yaml      # 6 scenarios
    ├── github.spec.yaml        # 5 scenarios
    ├── git.spec.yaml           # 6 scenarios
    ├── deepseek.spec.yaml      # 5 scenarios
    ├── kimi.spec.yaml          # 5 scenarios
    └── workspace.spec.yaml     # 6 scenarios
```

**Total: 177 scenarios across 23 components**

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

## Spec Maintenance Process

### When to Update Specs

- **Before** any behavior change (spec-first development)
- When adding new features or capabilities
- When fixing bugs that change observable behavior
- When adding new error handling or edge cases

### Update Workflow

1. **Update spec file**
   - Add or modify scenario(s)
   - Set `changed: YYYY-MM-DD` for modifications
   - Ensure `refs:` point to correct source locations

2. **Validate the spec**
   ```bash
   npx ajv-cli validate -s specs/schema.json -d specs/arkavo-edge/<component>.spec.yaml
   ```

3. **Update implementation**
   - Implement behavior changes
   - Add or update tests
   - Ensure code matches spec

4. **Update index.yaml**
   - Increment scenario_count if adding scenarios
   - Update last_updated date

5. **PR requirements**
   - Link spec changes in PR description
   - Include before/after behavior diff
   - Update version if breaking changes

### Review Requirements

| Change Type | Required Review |
|-------------|-----------------|
| Critical scenario changes | Security + Architect |
| New error scenarios | Tech Lead |
| Documentation only | Peer |
| Version bump | Architect |

### Versioning

Specs follow semantic versioning:

- **MAJOR**: Breaking behavior changes (e.g., new required field)
- **MINOR**: New scenarios without breaking changes
- **PATCH**: Documentation updates, typo fixes

### Deprecation Policy

```yaml
# Mark deprecated scenarios
- id: OLD-001
  deprecated: true
  deprecated_since: "2025-01-31"
  replacement: NEW-001
```

Deprecated scenarios are removed after 2 major versions.

### CI Enforcement

All PRs are checked by `.github/workflows/feature.yaml`:
- Schema validation
- Required field checking
- YAML syntax validation
- Statistics generation

**Specs must pass validation before merge.**
