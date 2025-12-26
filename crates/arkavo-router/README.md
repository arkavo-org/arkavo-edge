# arkavo-router

Intelligent model routing and quality-gated orchestration for Arkavo Edge.

## Features

- **Automated Task Classification**: High-speed task categorization (frontend, backend, security, etc.) in under 100ms.
- **Intelligent Routing**: Dynamic model selection across local (Gemma/Ministral) and cloud (Gemini/Claude) providers.
- **Quality Gate Integration**: Automated response validation with model escalation and retry logic.
- **Budget-Aware Orchestration**: Real-time cost optimization and routing based on available token runway.
- **Architect Mode**: Decomposes complex multi-step tasks into optimal sub-tasks routed to specific models.
- **Connectivity Awareness**: Transparent fallback to local models during offline or low-connectivity scenarios.
- **Performance Metrics**: Comprehensive tracking of routing decisions, costs, and realized savings.