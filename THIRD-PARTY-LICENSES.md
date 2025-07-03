# Third-Party Licenses

This project includes or embeds the following third-party software:

## idb_companion

**License:** MIT License  
**Copyright:** Copyright (c) Meta Platforms, Inc. and affiliates.  
**Source:** https://github.com/facebook/idb

```
MIT License

Copyright (c) Meta Platforms, Inc. and affiliates.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### Usage in This Project

idb_companion is optionally embedded in the macOS build of arkavo-edge to provide reliable iOS simulator UI automation. The binary is extracted at runtime for execution.

### Installation

If not embedded, idb_companion can be installed via Homebrew:
```bash
brew tap facebook/fb
brew install idb-companion
```

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