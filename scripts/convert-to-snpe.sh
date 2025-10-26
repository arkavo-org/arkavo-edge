#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

usage() {
    cat <<EOF
Convert models to SNPE DLC format for UNO Q deployment

Usage: $0 [OPTIONS] <model_path>

Arguments:
    model_path          Path to input model (.onnx, .gguf, .pt)

Options:
    -o, --output DIR    Output directory (default: ~/.cache/arkavo/models/dlc)
    -q, --quantize      Quantization mode: int8, fp16, none (default: int8)
    -t, --target        Target runtime: gpu, dsp, cpu (default: gpu)
    -b, --batch SIZE    Batch size for calibration (default: 32)
    -h, --help          Show this help message

Examples:
    # Convert ONNX model with INT8 quantization for GPU
    $0 models/gemma-3-270m.onnx

    # Convert with FP16 quantization for DSP
    $0 --quantize fp16 --target dsp models/vision-model.onnx

    # Convert without quantization
    $0 --quantize none models/model.onnx

Requirements:
    - SNPE SDK installed (set SNPE_ROOT environment variable)
    - Python 3.8+ with numpy, onnx packages
    - Input model in ONNX format (use ONNX export for PyTorch/TF models)
EOF
}

if [ -z "${SNPE_ROOT:-}" ]; then
    echo "Error: SNPE_ROOT environment variable not set"
    echo "Please set SNPE_ROOT to your SNPE SDK installation directory"
    echo "Example: export SNPE_ROOT=/opt/snpe"
    exit 1
fi

OUTPUT_DIR="$HOME/.cache/arkavo/models/dlc"
QUANTIZE="int8"
TARGET="gpu"
BATCH_SIZE=32

while [[ $# -gt 0 ]]; do
    case $1 in
        -o|--output)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        -q|--quantize)
            QUANTIZE="$2"
            shift 2
            ;;
        -t|--target)
            TARGET="$2"
            shift 2
            ;;
        -b|--batch)
            BATCH_SIZE="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        -*)
            echo "Unknown option: $1"
            usage
            exit 1
            ;;
        *)
            MODEL_PATH="$1"
            shift
            ;;
    esac
done

if [ -z "${MODEL_PATH:-}" ]; then
    echo "Error: model_path argument required"
    usage
    exit 1
fi

if [ ! -f "$MODEL_PATH" ]; then
    echo "Error: Model file not found: $MODEL_PATH"
    exit 1
fi

mkdir -p "$OUTPUT_DIR"

MODEL_NAME=$(basename "$MODEL_PATH" | sed 's/\.[^.]*$//')
ONNX_PATH="$OUTPUT_DIR/${MODEL_NAME}.onnx"
DLC_PATH="$OUTPUT_DIR/${MODEL_NAME}.dlc"

echo "=== SNPE Model Conversion Pipeline ==="
echo "Input:       $MODEL_PATH"
echo "Output:      $DLC_PATH"
echo "Quantize:    $QUANTIZE"
echo "Target:      $TARGET"
echo "Batch size:  $BATCH_SIZE"
echo

if [[ "$MODEL_PATH" == *.onnx ]]; then
    ONNX_PATH="$MODEL_PATH"
    echo "✓ Input is already ONNX format"
else
    echo "Error: Only ONNX models are currently supported"
    echo "Please export your model to ONNX format first"
    echo "PyTorch: torch.onnx.export(model, dummy_input, 'model.onnx')"
    echo "TensorFlow: tf2onnx.convert.from_keras(model, output_path='model.onnx')"
    exit 1
fi

SNPE_BIN="$SNPE_ROOT/bin/x86_64-linux-clang"
if [ ! -d "$SNPE_BIN" ]; then
    echo "Error: SNPE binaries not found at: $SNPE_BIN"
    exit 1
fi

export PATH="$SNPE_BIN:$PATH"
export LD_LIBRARY_PATH="$SNPE_ROOT/lib/x86_64-linux-clang:${LD_LIBRARY_PATH:-}"

echo "Step 1: Converting ONNX to DLC..."
snpe-onnx-to-dlc \
    --input_network "$ONNX_PATH" \
    --output_path "$DLC_PATH" \
    2>&1 | tee "$OUTPUT_DIR/${MODEL_NAME}_conversion.log"

if [ ! -f "$DLC_PATH" ]; then
    echo "Error: DLC conversion failed"
    cat "$OUTPUT_DIR/${MODEL_NAME}_conversion.log"
    exit 1
fi

echo "✓ DLC conversion complete: $DLC_PATH"

if [ "$QUANTIZE" != "none" ]; then
    echo
    echo "Step 2: Quantizing model to $QUANTIZE..."

    QUANT_DLC="$OUTPUT_DIR/${MODEL_NAME}_${QUANTIZE}.dlc"

    if [ "$QUANTIZE" = "int8" ]; then
        echo "Generating calibration data (batch size: $BATCH_SIZE)..."
        CALIB_DATA="$OUTPUT_DIR/${MODEL_NAME}_calibration.raw"

        python3 <<EOF
import numpy as np
import struct

batch_size = $BATCH_SIZE
input_shape = (batch_size, 3, 224, 224)
data = np.random.randn(*input_shape).astype(np.float32)

with open('$CALIB_DATA', 'wb') as f:
    f.write(data.tobytes())

print(f"Generated calibration data: {input_shape}")
EOF

        snpe-dlc-quantize \
            --input_dlc "$DLC_PATH" \
            --output_dlc "$QUANT_DLC" \
            --input_list "$CALIB_DATA" \
            --optimizations cle \
            2>&1 | tee "$OUTPUT_DIR/${MODEL_NAME}_quantize.log"

    elif [ "$QUANTIZE" = "fp16" ]; then
        snpe-dlc-quantize \
            --input_dlc "$DLC_PATH" \
            --output_dlc "$QUANT_DLC" \
            --use_enhanced_quantizer \
            --weights_bitwidth 16 \
            2>&1 | tee "$OUTPUT_DIR/${MODEL_NAME}_quantize.log"
    fi

    if [ -f "$QUANT_DLC" ]; then
        mv "$QUANT_DLC" "$DLC_PATH"
        echo "✓ Quantization complete: $DLC_PATH"
    else
        echo "Warning: Quantization failed, using unquantized model"
    fi
fi

echo
echo "Step 3: Validating DLC model..."
snpe-dlc-info --input_dlc "$DLC_PATH" \
    2>&1 | tee "$OUTPUT_DIR/${MODEL_NAME}_info.txt"

DLC_SIZE=$(du -h "$DLC_PATH" | cut -f1)
echo
echo "=== Conversion Complete ==="
echo "DLC file: $DLC_PATH"
echo "Size:     $DLC_SIZE"
echo "Target:   $TARGET"
echo "Quantize: $QUANTIZE"
echo
echo "Next steps:"
echo "1. Copy DLC to UNO Q: scp $DLC_PATH uno-q:/opt/arkavo/models/"
echo "2. Set ARKAVO_MODEL_PATH=/opt/arkavo/models/$(basename "$DLC_PATH")"
echo "3. Set ARKAVO_SNPE_RUNTIME=$TARGET"
echo "4. Run: arkavo chat --model snpe --prompt 'test'"
