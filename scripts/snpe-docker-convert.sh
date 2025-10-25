#!/bin/bash
set -e

# SNPE ONNX to DLC Converter using Docker
# Runs on any platform (macOS, Linux, Windows) via Docker

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SNPE_SDK_PATH="$PROJECT_ROOT/vendor/qairt/2.39.0.250926"

# Check if SNPE SDK exists
if [ ! -d "$SNPE_SDK_PATH" ]; then
    echo "Error: SNPE SDK not found at $SNPE_SDK_PATH"
    echo "Please download SNPE SDK 2.39.0 from:"
    echo "https://developer.qualcomm.com/software/qualcomm-neural-processing-sdk"
    exit 1
fi

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo "Error: Docker is not running"
    echo "Please start Docker Desktop"
    exit 1
fi

# Parse arguments
ONNX_FILE="${1:-}"
OUTPUT_DLC="${2:-output.dlc}"

if [ -z "$ONNX_FILE" ]; then
    echo "Usage: $0 <input.onnx> [output.dlc]"
    echo ""
    echo "Example:"
    echo "  $0 ~/.cache/arkavo/models/onnx/qwen3-0.6b_static_seq64.onnx qwen3-0.6b.dlc"
    exit 1
fi

if [ ! -f "$ONNX_FILE" ]; then
    echo "Error: ONNX file not found: $ONNX_FILE"
    exit 1
fi

ONNX_BASENAME="$(basename "$ONNX_FILE")"
OUTPUT_BASENAME="$(basename "$OUTPUT_DLC")"
OUTPUT_DIR="$(dirname "$OUTPUT_DLC")"

# Make output dir absolute
if [[ "$OUTPUT_DIR" != /* ]]; then
    OUTPUT_DIR="$PWD/$OUTPUT_DIR"
fi
mkdir -p "$OUTPUT_DIR"

echo "=== SNPE ONNX to DLC Conversion ==="
echo "Input:  $ONNX_FILE"
echo "Output: $OUTPUT_DIR/$OUTPUT_BASENAME"
echo ""

# Build Docker image
echo "Building Docker image..."
docker build -t snpe-converter - <<'EOF'
FROM ubuntu:22.04

# Install dependencies
RUN apt-get update && apt-get install -y \
    python3 \
    python3-pip \
    && rm -rf /var/lib/apt/lists/*

# SNPE SDK will be mounted at /snpe
WORKDIR /workspace

CMD ["/bin/bash"]
EOF

# Run conversion in Docker
echo ""
echo "Running conversion..."
docker run --rm \
    -v "$SNPE_SDK_PATH:/snpe:ro" \
    -v "$(dirname "$ONNX_FILE"):/input:ro" \
    -v "$OUTPUT_DIR:/output" \
    snpe-converter \
    bash -c "
        export SNPE_ROOT=/snpe
        export PYTHONPATH=/snpe/lib/python
        export PATH=/snpe/bin/x86_64-linux-clang:\$PATH

        echo 'Converting ONNX to DLC...'
        python3 /snpe/bin/x86_64-linux-clang/snpe-onnx-to-dlc \
            -i /input/$ONNX_BASENAME \
            -o /output/$OUTPUT_BASENAME

        echo ''
        echo '=== DLC Info ==='
        /snpe/bin/x86_64-linux-clang/snpe-dlc-info \
            -i /output/$OUTPUT_BASENAME || true

        echo ''
        echo '✓ Conversion complete!'
        ls -lh /output/$OUTPUT_BASENAME
    "

echo ""
echo "=== Success ==="
echo "DLC file: $OUTPUT_DIR/$OUTPUT_BASENAME"
echo ""
echo "Next steps:"
echo "  1. Transfer to UNO Q:"
echo "     adb push $OUTPUT_DIR/$OUTPUT_BASENAME /data/local/tmp/"
echo ""
echo "  2. Run inference:"
echo "     adb shell 'snpe-net-run --container /data/local/tmp/$OUTPUT_BASENAME'"
