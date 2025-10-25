#!/usr/bin/env python3
"""
Convert ONNX model with dynamic shapes to static shapes for SNPE.

Replaces all dynamic dimensions (dim_param) with fixed values suitable
for edge deployment (batch=1, seq=64).
"""
import argparse
import onnx
from onnx import helper, shape_inference
from pathlib import Path


def fix_dynamic_shapes(model_path, output_path, batch_size=1, seq_length=64):
    """Convert dynamic shapes to static shapes in ONNX model."""

    print(f"Loading model: {model_path}")
    model = onnx.load(str(model_path))

    # Define dimension mappings
    dim_map = {
        "batch_size": batch_size,
        "sequence_length": seq_length,
        "past_sequence_length": 0,  # No KV cache for initial inference
        "total_sequence_length": seq_length,
    }

    print(f"\nDimension mapping:")
    for dim_param, dim_value in dim_map.items():
        print(f"  {dim_param} -> {dim_value}")

    # Fix input shapes
    print(f"\nFixing input shapes...")
    for input_tensor in model.graph.input:
        shape = input_tensor.type.tensor_type.shape
        for i, dim in enumerate(shape.dim):
            if dim.HasField("dim_param") and dim.dim_param in dim_map:
                old_param = dim.dim_param
                new_value = dim_map[dim.dim_param]
                dim.Clear()
                dim.dim_value = new_value
                print(f"  {input_tensor.name}[{i}]: {old_param} -> {new_value}")

    # Fix output shapes
    print(f"\nFixing output shapes...")
    for output_tensor in model.graph.output:
        shape = output_tensor.type.tensor_type.shape
        for i, dim in enumerate(shape.dim):
            if dim.HasField("dim_param") and dim.dim_param in dim_map:
                old_param = dim.dim_param
                new_value = dim_map[dim.dim_param]
                dim.Clear()
                dim.dim_value = new_value
                print(f"  {output_tensor.name}[{i}]: {old_param} -> {new_value}")

    # Run shape inference to propagate fixed shapes through the graph
    print(f"\nRunning shape inference...")
    try:
        model = shape_inference.infer_shapes(model)
        print(f"  ✓ Shape inference successful")
    except Exception as e:
        print(f"  ⚠️ Shape inference failed (may still work): {e}")

    # Save fixed model
    print(f"\nSaving fixed model: {output_path}")
    output_path = Path(output_path)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    onnx.save(model, str(output_path))

    file_size_mb = output_path.stat().st_size / (1024 * 1024)
    print(f"  ✓ Saved ({file_size_mb:.1f} MB)")

    # Validate
    print(f"\nValidating fixed model...")
    try:
        onnx.checker.check_model(model)
        print(f"  ✓ Model is valid")
    except Exception as e:
        print(f"  ⚠️ Validation warning: {e}")

    # Print final input/output shapes
    print(f"\nFinal input shapes:")
    for input_tensor in model.graph.input:
        shape = [d.dim_value for d in input_tensor.type.tensor_type.shape.dim]
        print(f"  {input_tensor.name}: {shape}")

    print(f"\nFinal output shapes:")
    for output_tensor in model.graph.output:
        shape = [d.dim_value if d.HasField("dim_value") else d.dim_param
                 for d in output_tensor.type.tensor_type.shape.dim]
        print(f"  {output_tensor.name}: {shape}")

    return output_path


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Fix ONNX dynamic shapes to static")
    parser.add_argument("input", help="Input ONNX model path")
    parser.add_argument("output", help="Output ONNX model path")
    parser.add_argument("--batch-size", type=int, default=1, help="Batch size")
    parser.add_argument("--seq-length", type=int, default=64, help="Sequence length")

    args = parser.parse_args()

    fix_dynamic_shapes(
        args.input,
        args.output,
        batch_size=args.batch_size,
        seq_length=args.seq_length,
    )
