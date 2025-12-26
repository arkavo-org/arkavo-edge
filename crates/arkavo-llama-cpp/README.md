# arkavo-llama-cpp

High-level Rust wrapper for the llama.cpp inference engine, optimized for edge deployment.

## Features

- **Automated Acceleration**: Intelligent detection of GPU acceleration (Metal, CUDA, Vulkan) with seamless CPU fallback.
- **Multi-Format Support**: Native support for modern chat templates including Gemma-3, Mistral V3, and Qwen3.
- **Advanced Sampling**: Fully configurable sampling chains with Top-K, Top-P, temperature, and greedy selection.
- **Constrained Decoding**: Integrated logit bias support for formal language generation and TØRG compliance.
- **Memory Optimization**: Adaptive KV cache management and context scaling for resource-constrained devices.
- **Platform Optimized**: Native performance tuning for Apple Silicon and low-power ARM devices like Raspberry Pi.
- **Multimodal Support**: Ready for vision-language models with dedicated multimodal processing.
