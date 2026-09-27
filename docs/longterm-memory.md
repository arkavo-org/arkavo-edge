# Long-Term Memory

This document outlines the core architectural principles and long-term vision for Arkavo Edge. It is a synthesis of the key design documents that guide the project's development.

## 1. Core System Architecture

### 1.1. Agent and Gateway Roles
- The **AG-UI Gateway** is an orchestrator agent, responsible for managing and visualizing the agent mesh. It does not have its own LLM.
- Each **headless agent** is an autonomous entity that manages its own LLM connection as defined in its `AGENTS.md` configuration.

### 1.2. AI-Driven, Zero-Configuration Principle
- The system is designed to be **zero-configuration**. Users should be able to run it without manual setup.
- **AI-Driven Configuration**: Instead of static files, the system uses an AI-driven, interactive process. The agent discovers its environment and asks the user for guidance when necessary.
- **Memory Storage**: All configurations (e.g., Ollama server URLs) are stored in the `arkavo-memory` crate, not in environment variables or config files, enabling dynamic, persistent configuration.

### 1.3. Dependency Management
- The macOS crate ships as a single binary. iOS automation uses Xcode, simctl, AppleScript, XCTest, and the AXP harness rather than embedding third-party binaries.

## 2. Agent Communication & Dataflow

### 2.1. A2A Protocol: Secure, Session-Based Communication
- The Agent-to-Agent (A2A) protocol is a **full-duplex, session-based system** built on JSON-RPC for robust and stateful communication.
- **Session Lifecycle**: Interactions follow a clear lifecycle (`chat_open`, `chat_send`, `chat_stream`, `chat_close`), allowing for stateful conversations.
- **Security**: All communication is secured with **mTLS** by default, using `rustls` to avoid OpenSSL dependencies.
- **Discovery**: Agents find each other automatically on the network using **mDNS** and **DNS-SRV**.

### 2.2. Dataflow Blueprints: Declarative Agent Workflows
- Complex agent workflows are defined using declarative **Dataflow Blueprints** (in JSON or YAML).
- These blueprints define a pipeline of nodes (`source`, `transform`, `router`, `sink`) and the links between them.
- This allows for the creation of sophisticated, multi-agent, multi-LLM processing pipelines (e.g., classification and routing, parallel analysis).

## 3. LLM and Model Management

### 3.1. Multi-Provider LLM Router
- The system is built around a **multi-provider LLM architecture** that supports multiple LLM providers (Ollama, OpenAI, Anthropic) simultaneously.
- A central **LLM Router** dynamically selects the best provider/model for a given request based on capabilities (vision, function calling), cost, and availability.
- The architecture is designed to be extensible, allowing new providers to be added with minimal friction.
- **IMPORTANT**: Never hardcode specific models in component code. Always route LLM tasks through the Router with descriptive task hints (e.g., "Analyze system health data and generate structured JSON response"). This ensures the system adapts as models come and go.

### 3.2. Local LLM Support
- Arkavo Edge supports running language models locally using llama.cpp for GGUF model inference with privacy-first, offline-capable operation.
- **Hardware Acceleration**: It automatically uses Metal Performance Shaders on macOS and CPU-based inference on other platforms.
- **Model Download Manager**: A secure download manager acquires GGUF models from Hugging Face, performs SHA-256 verification, and ensures license compliance.
- **Model Agnostic Architecture**: Components request LLM capabilities through the router, not specific models. The router selects appropriate models (e.g., 270M for simple tasks, 4B for complex analysis, cloud for specialized needs) based on task requirements.

### 3.3. Health Monitoring Architecture
- **Intelligent Health Monitoring**: The system uses local LLM (via Router) to analyze component health and decide when to notify users.
- **Router-Based Selection**: Health analysis tasks are routed through the Router with descriptive hints, allowing intelligent model selection without hardcoding.
- **Auto-Fix First**: The system attempts to automatically fix transient issues (connectivity, cache, restarts) before alerting users.
- **User Notification Policy**: Only notify users for issues requiring human intervention (API key errors, persistent failures, security issues).
- **Minimal Fallback**: Simple rule-based fallback only checks healthy/degraded/unhealthy if LLM analysis fails completely.

## 4. iOS Testing & Automation Architecture

### 4.1. The XCTest Bridge: Reliable, High-Performance UI Automation
- The primary mechanism for iOS UI automation is the **XCTest Bridge**. This replaces fragile, coordinate-based AppleScript automation.
- **Architecture**: A Rust MCP server communicates with a Swift XCTest runner on the simulator via a high-performance **Unix Socket**.
- **Benefits**: This approach requires no special accessibility permissions, works regardless of simulator window position, and allows for reliable, text-based element finding.

### 4.2. The FFI Bridge: Deep State Inspection
- For sub-50ms test execution and deep app inspection, Arkavo uses a Foreign Function Interface (FFI) bridge.
- **Architecture**: The Rust test harness communicates with the iOS app through a C interface, allowing direct manipulation of app state without going through the UI.
- **Capabilities**: This enables powerful features like creating and restoring full application state snapshots, direct function calls, and runtime object manipulation.

### 4.3. Zero-Touch Intelligent Testing
- The long-term vision is for **zero-touch intelligent testing**.
- **Runtime Injection**: Arkavo will use dynamic instrumentation (e.g., `DYLD_INSERT_LIBRARIES`) to inject a test harness into any application at runtime without modifying its source code.
- **AI-Powered Analysis**: The injected harness, combined with AI analysis of the codebase, will allow Arkavo to discover bugs, generate property tests, and explore edge cases autonomously.

## 5. Git Integration

### 5.1. Secure and Safe by Default
- The Git integration is built using `git2` but with `rustls` to avoid the OpenSSL dependency, ensuring portability.
- **RepoGuard**: A transaction wrapper provides atomic commits with automatic rollback on failure, preventing repository corruption.
- **Pre-commit Validation**: The system can be configured to run `cargo fmt`, `clippy`, and other checks before finalizing a commit.
- **Path Sanitization**: All file paths are sanitized to prevent directory traversal attacks.
