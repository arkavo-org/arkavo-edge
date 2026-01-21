# AGENTS.md

## backend-agent
purpose: Implement backend features including REST APIs, database schemas, and business logic
listen:  0.0.0.0:8361

# The backend agent handles:
# - REST API endpoint implementation
# - Database schema design and migrations
# - Business logic and validation
# - Authentication/authorization
# - Integration tests for APIs

# Enable mDNS for mesh discovery
discovery:
  mdns: true
