# arkavo-snpe

Qualcomm SNPE backend for hardware-accelerated inference on Arkavo Edge.

## Features

- **Multi-Target Acceleration**: Native support for GPU (Adreno 702), DSP (Hexagon), and CPU inference.
- **Dynamic Loading**: Portable runtime implementation using dlopen for zero build-time SDK dependencies.
- **UNO Q Optimization**: Specialized performance tuning for the Arduino UNO Q (QRB2210) platform.
- **Automatic Fallback**: Intelligent detection of hardware capabilities with graceful fallback to CPU.
- **Deep Learning Containers**: Native support for loading and executing Qualcomm DLC model formats.
- **Low-Latency Inference**: Sub-50ms inference targets for quantized models on supported edge hardware.