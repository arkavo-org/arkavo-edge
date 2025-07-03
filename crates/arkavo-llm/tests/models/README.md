# Test Models Directory

This directory contains quantized model files used for integration testing.

## TinyLlama Model

For testing, we use the TinyLlama 1B model in Q4_K_M quantization format.

### Download Instructions

1. Download the model from Hugging Face:
   ```bash
   wget https://huggingface.co/TheBloke/TinyLlama-1.1B-Chat-v1.0-GGUF/resolve/main/tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf
   ```

2. Verify the SHA-256 checksum:
   ```bash
   echo "c89091545b7e0c398a8cfcfbe27c1108d4864c8e48c39fa6de5033bb4b469eb8  tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf" | sha256sum -c
   ```

3. Rename to match test expectations:
   ```bash
   mv tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf tinyllama-1b.Q4_K_M.gguf
   ```

## Git LFS Setup

To track model files with Git LFS:

1. Install Git LFS if not already installed:
   ```bash
   git lfs install
   ```

2. Track GGUF files:
   ```bash
   git lfs track "*.gguf"
   ```

3. Add and commit:
   ```bash
   git add .gitattributes
   git add tinyllama-1b.Q4_K_M.gguf
   git commit -m "Add TinyLlama test model via Git LFS"
   ```

## Model Size

- `tinyllama-1b.Q4_K_M.gguf`: ~638 MB

## License

TinyLlama is licensed under Apache 2.0. See the model card on Hugging Face for details.