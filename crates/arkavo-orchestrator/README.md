# arkavo-github-orchestrator

GitHub webhook orchestration and automated issue assignment for Arkavo agents.

## Overview

This crate provides the foundation for automated GitHub issue management through AI agents. It handles webhook reception, GitHub App authentication, and intelligent issue analysis.

## Features

### Webhook Server
- **Secure Webhook Reception**: HMAC-SHA256 signature validation
- **Event Routing**: Type-safe event deserialization and internal routing
- **Health Monitoring**: Built-in health check endpoint
- **CORS Support**: Configurable CORS for development

### GitHub App Authentication
- **JWT Generation**: RS256 tokens for GitHub App authentication
- **Token Caching**: Automatic installation token caching with renewal
- **Installation Discovery**: Find installations by repository owner
- **Rate Limit Friendly**: 5000 req/hr with GitHub App tokens

### Issue Analysis
- **Type Classification**: Automatically categorizes issues (Bug, Feature, Documentation, etc.)
- **Complexity Assessment**: Trivial, Simple, Moderate, or Complex
- **Technology Detection**: Identifies required technologies (Rust, Python, Docker, etc.)
- **Capability Mapping**: Determines required agent capabilities
- **Budget Estimation**: Token budget allocation based on complexity

## Usage

### CLI Command

Start the orchestrator server from the command line:

```bash
# Using environment variables
export ARKAVO_GITHUB_WEBHOOK_SECRET="your-webhook-secret"
export ARKAVO_GITHUB_APP_ID="123456"
export ARKAVO_GITHUB_APP_PRIVATE_KEY="$(cat private-key.pem)"

arkavo orchestrator start

# Or pass arguments directly
arkavo orchestrator start --port 3000 \
  --webhook-secret "your-secret" \
  --app-id "123456" \
  --private-key "$(cat private-key.pem)"

# Check configuration
arkavo orchestrator config

# View status
arkavo orchestrator status
```

### Creating a Webhook Server (Programmatically)

```rust
use arkavo_github_orchestrator::WebhookServer;

let secret = std::env::var("GITHUB_WEBHOOK_SECRET")?;
let (server, mut event_rx) = WebhookServer::new(secret);

// Start receiving events
tokio::spawn(async move {
    while let Some(event) = event_rx.recv().await {
        // Process events
    }
});

// Start the server
let app = server.router();
axum::Server::bind(&"0.0.0.0:3000".parse()?)
    .serve(app.into_make_service())
    .await?;
```

### GitHub App Authentication

```rust
use arkavo_github_orchestrator::GitHubApp;

let app_id = 123456;
let private_key = std::fs::read_to_string("private-key.pem")?;

let github_app = GitHubApp::new(app_id, &private_key)?;

// Get installation for a repository owner
let installation_id = github_app
    .find_installation_by_owner("arkavo-org")
    .await?
    .expect("Installation not found");

// Get installation token (cached automatically)
let token = github_app.get_installation_token(installation_id).await?;
```

### Analyzing Issues

```rust
use arkavo_github_orchestrator::{IssueAnalyzer, IssueEvent};

let analysis = IssueAnalyzer::analyze(&issue_event);

println!("Type: {:?}", analysis.issue_type);
println!("Complexity: {:?}", analysis.complexity);
println!("Technologies: {:?}", analysis.technologies);
println!("Required capabilities: {:?}", analysis.required_capabilities);
println!("Token budget: {}", analysis.estimated_tokens);
```

## Issue Complexity

Complexity levels determine token budgets and routing decisions:

- **Trivial** (10k tokens): Typos, simple docs updates
- **Simple** (50k tokens): Small bugs, dependency bumps
- **Moderate** (200k tokens): New features, refactors
- **Complex** (500k tokens): Architecture changes, multi-file refactors

## Environment Variables

- `GITHUB_WEBHOOK_SECRET`: Secret for validating webhook signatures
- `GITHUB_APP_ID`: GitHub App ID
- `GITHUB_APP_PRIVATE_KEY_PATH`: Path to GitHub App private key PEM file

## Security

- All webhook payloads are verified using HMAC-SHA256
- GitHub App private keys should be stored securely
- Installation tokens are automatically renewed before expiry
- Least-privilege permissions via GitHub App configuration

## Integration

This crate integrates with:
- **arkavo-protocol**: Agent registry and task orchestration
- **arkavo-memory**: Event persistence and state recovery
- **arkavo-budget**: Token budget tracking
- **arkavo-events**: Event correlation and tracking

## Status

**Phase 1-2: Complete** ✅
- ✅ Webhook server with HMAC-SHA256 verification
- ✅ GitHub App authentication with JWT
- ✅ Issue analysis and classification
- ✅ Intelligent routing (4 execution strategies)
- ✅ Agent assignment and registry
- ✅ Cognitive engine with task execution
- ✅ Progress tracking and status updates
- ✅ Event tracking and metrics
- ✅ CLI command interface

**Phase 3-4: In Progress** 🚧
- Multi-agent task coordination
- Result aggregation across agents
- End-to-end integration testing
- Production deployment guides
