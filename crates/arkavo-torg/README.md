# arkavo-torg

TØR-G constrained decoding integration for Arkavo Edge agents.

## Features

- **Constrained Decoding**: Formal logit masking to ensure LLM output strictly adheres to TØRG policy structures.
- **Multi-Model Support**: Native token mapping and sampler support for Qwen3 and Ministral-3B models.
- **Direct Inference Integration**: Seamless integration with the llama.cpp sampler for local model enforcement.
- **Formal-Language Bridge**: Connects high-level natural language requests with formally verified boolean graphs.
- **High-Speed Masking**: Optimized mask generation and feeding logic for low-latency constrained inference.
- **Graph Extraction**: Automated conversion of sampled tokens back into executable TØRG policy graphs.
