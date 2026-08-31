# Third-Party Licenses

This project includes or embeds the following third-party software:

## Gemma Models

**License:** Gemma Terms of Use  
**Copyright:** Copyright 2024 Google LLC  
**Source:** https://ai.google.dev/gemma/terms

When Gemma models are downloaded and used by Arkavo Edge, they are subject to the Gemma Terms of Use. The models include the following notice:

```
NOTICE

This repository contains pre-trained model weights for Gemma models.
These model weights are licensed for use under the Gemma Terms of Use:
https://ai.google.dev/gemma/terms
```

### Usage in This Project

Gemma models can be optionally downloaded and used for local inference in Arkavo Edge. When downloaded, the models and their Notice.txt file are stored in the user's cache directory. The models are:

- **gemma-2-2b-it-GGUF**: 2 billion parameter instruction-tuned model
- **gemma-2-9b-it-GGUF**: 9 billion parameter instruction-tuned model
- **gemma-2-27b-it-GGUF**: 27 billion parameter instruction-tuned model

### Compliance

When distributing Arkavo Edge with bundled Gemma models, the Notice.txt file must be included. The local inference system automatically manages this compliance by:

1. Downloading the Notice.txt file along with model files
2. Storing it in the model directory
3. Including it in any redistribution of models

## Qwen Models

**License:** Apache License 2.0

**Source:** https://huggingface.co/Qwen

Qwen models are optionally downloaded for local inference, and Qwen3.5-0.8B is
the base for the DLP sentinel classifier distilled in the knowledge-pack
pipeline. Apache-2.0 places no restriction on derivatives or redistribution, so
a fine-tuned sentinel may be shipped inside a sealed pack.

- **Qwen3.5-0.8B**: 0.8 billion parameters; local inference and the sentinel base
- **Qwen3.5-9B**, **Qwen3.5-27B**, **Qwen3.6-35B-A3B**: local inference

GGUF quantizations are pulled from community requants (`unsloth/*-GGUF`), also
Apache-2.0. Weights fine-tuned by the distillation pipeline are derivative works
of the official Qwen weights and inherit Apache-2.0; each shipped pack records
its base model and revision in the pack manifest.
