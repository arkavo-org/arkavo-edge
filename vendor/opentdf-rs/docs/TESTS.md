# OpenTDF-RS Tests with MCP Approach

This document outlines natural language tests for OpenTDF-RS using the Model Context Protocol (MCP) approach instead of traditional Gherkin BDD syntax.

## About MCP Testing

The Model Context Protocol (MCP) approach transforms how we write and execute behavior-driven tests:

- Tests are written in natural language without rigid syntax requirements
- Test execution happens through direct interaction with the MCP server
- The MCP server provides rich context about application capabilities
- AI testing agents can interpret natural language test descriptions and execute them against the actual system
- Tests automatically adapt as the application evolves

## Running Tests with Claude Integration

OpenTDF-RS provides MCP integration with Claude for executing tests:

```bash
# Start Claude with MCP server
claude --mcp="cargo run -p opentdf-mcp-server"

# Then in Claude, run:
/mcp opentdf attribute_define {"namespace": "gov.example", "name": "clearance", "values": ["public", "confidential", "secret", "top-secret"], "hierarchy": [{"value": "top-secret", "inherits_from": "secret"}, {"value": "secret", "inherits_from": "confidential"}, {"value": "confidential", "inherits_from": "public"}]}

# Alternative: Run test script
node tools/test-mcp.js
```

## Attribute-Based Access Control Tests

### Basic Attribute Policy Creation

```
Test: Creating a TDF with attribute-based access policy

Given I have a sensitive document
When I create a TDF with an attribute-based policy requiring "clearance:SECRET" attribute
Then the TDF should contain a cryptographically bound policy
And the policy should include the "clearance:SECRET" attribute requirement
And the policy binding should be properly signed
```

### Multi-Attribute Policy Enforcement

```
Test: Enforcing policies with multiple attribute requirements

Given I have a TDF protected with multiple attribute requirements
| Attribute             | Operator | Value        |
| department            | equals   | finance      |
| clearance             | minimumOf| CONFIDENTIAL |
| location              | in       | [USA, CANADA]|

When a user with the following attributes attempts to access:
| Attribute             | Value        |
| department            | finance      |
| clearance             | SECRET       |
| location              | USA          |

Then the user should be granted access to the TDF content

When a user with the following attributes attempts to access:
| Attribute             | Value        |
| department            | engineering  |
| clearance             | TOP_SECRET   |
| location              | USA          |

Then the user should be denied access
And an access denial event should be logged
```

### Time-Constrained Access

```
Test: Enforcing time-based constraints in attribute policies

Given I have a document with sensitive quarterly results
When I create a TDF with policy requiring "department:finance" AND time constraint "valid_after:2023-04-01T09:00:00Z"
Then the TDF should contain a time-constrained attribute policy

When a finance user attempts access before the valid date
Then access should be denied
And a time constraint violation should be logged

When a finance user attempts access after the valid date
Then access should be granted
And the access event should be recorded in the audit log
```

### Hierarchical Attribute Evaluation

```
Test: Supporting hierarchical attribute structures

Given I have defined a hierarchical attribute structure for "clearance":
| Level | Value          | Inherits From   |
| 1     | PUBLIC         | none            |
| 2     | SENSITIVE      | PUBLIC          |
| 3     | CONFIDENTIAL   | SENSITIVE       |
| 4     | SECRET         | CONFIDENTIAL    |
| 5     | TOP_SECRET     | SECRET          |

When I create a TDF requiring "clearance:CONFIDENTIAL"
And a user with "clearance:SECRET" attempts access
Then access should be granted
Because SECRET inherits all privileges of CONFIDENTIAL

When a user with "clearance:SENSITIVE" attempts access
Then access should be denied
Because SENSITIVE does not include CONFIDENTIAL privileges
```

### Environmental Attribute Verification

```
Test: Evaluating environmental attributes

Given I have a TDF with a policy requiring "network:corporate" environmental attribute
When a user on the corporate network attempts access
Then the environmental attribute should be verified
And access should be granted

When the same user attempts access from a public network
Then the environmental attribute verification should fail
And access should be denied
```

### Complex Policy Logic

```
Test: Supporting complex logical operations in policies

Given I have a TDF with policy: "(department:legal OR department:compliance) AND clearance:CONFIDENTIAL AND NOT region:APAC"
When a legal user with CONFIDENTIAL clearance from EMEA attempts access
Then access should be granted

When a legal user with CONFIDENTIAL clearance from APAC attempts access
Then access should be denied because of the NOT region:APAC restriction

When a marketing user with CONFIDENTIAL clearance from EMEA attempts access
Then access should be denied because they lack the required department attribute
```

### Attribute Resolution and Caching

```
Test: Resolving and caching user attributes

Given I have a TDF requiring specific security attributes
And an attribute provider service is available

When a user attempts access for the first time
Then the system should query the attribute provider for user attributes
And cache the verified attributes with appropriate TTL

When the same user attempts access within the cache TTL period
Then the system should use the cached attributes without querying the provider

When the cache TTL expires
Then the system should re-verify attributes on next access attempt
```

### Policy Update and Versioning

```
Test: Handling policy updates

Given I have a TDF with an attribute-based policy
When I update the policy with additional attribute requirements
Then a new policy version should be created
And the updated policy should be cryptographically bound to the TDF

When a user attempts access with attributes satisfying the original policy only
Then access should be denied
And the policy version mismatch should be logged

When a user with attributes satisfying the new policy attempts access
Then access should be granted
```

### Audit and Compliance

```
Test: Comprehensive attribute access logging

Given I have a TDF with attribute-based access controls
When various access attempts occur
Then the audit log should record:
  - The requesting entity identifier
  - The complete set of attributes presented
  - The attribute sources and verification status
  - The evaluation result for each attribute in the policy
  - The final access decision with timestamp
  - Any policy version information

When a compliance officer generates an access report
Then the report should include all relevant attribute evaluations
And provide evidence of proper policy enforcement
```

*Implementation Note:* This test has been implemented as a comprehensive JavaScript test script in `tools/audit-logging-test.js`. The script performs multiple access attempts with different user attributes and generates compliance reports in the `tools/reports` directory. Run it with `node tools/audit-logging-test.js` to verify audit logging functionality.