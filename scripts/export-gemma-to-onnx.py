#!/usr/bin/env python3
"""
Export Gemma-3-270M model to ONNX format for SNPE conversion.

Constraints for SNPE compatibility:
- Static shapes only (no dynamic axes)
- Small context window (64-128 tokens)
- Simplified model (no KV cache, no advanced optimizations)
- Float32 precision for conversion (quantization happens in SNPE)
"""
import argparse
import torch
import os
from pathlib import Path
from transformers import AutoModelForCausalLM, AutoTokenizer, AutoConfig


class SimplifiedGemmaWrapper(torch.nn.Module):
    """Wrapper to simplify Gemma model for SNPE export."""

    def __init__(self, model, max_length):
        super().__init__()
        self.model = model
        self.max_length = max_length

    def forward(self, input_ids):
        """Simplified forward pass with static shapes."""
        # Create attention mask (all ones for valid tokens)
        attention_mask = torch.ones_like(input_ids)

        # Disable use_cache to avoid KV cache complexity
        outputs = self.model(
            input_ids=input_ids,
            attention_mask=attention_mask,
            use_cache=False,
            return_dict=True,
        )

        # Return only logits (no past_key_values or other outputs)
        return outputs.logits


def export_gemma_to_onnx(
    model_name: str = "google/gemma-3-270m-it",
    output_path: str = None,
    seq_length: int = 64,
    batch_size: int = 1,
):
    """Export Gemma model to ONNX format with SNPE constraints."""

    if output_path is None:
        output_path = Path.home() / ".cache" / "arkavo" / "models" / "onnx" / f"gemma-3-270m_seq{seq_length}.onnx"

    output_path = Path(output_path)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    print(f"Loading model: {model_name}")
    print(f"SNPE export constraints:")
    print(f"  - Static shapes: batch={batch_size}, seq_length={seq_length}")
    print(f"  - No KV cache")
    print(f"  - No dynamic axes")
    print(f"  - Float32 precision (INT8 quantization in SNPE)")

    # Load config and disable features incompatible with SNPE
    config = AutoConfig.from_pretrained(model_name)

    tokenizer = AutoTokenizer.from_pretrained(model_name)

    # Load model in float32 (SNPE quantization will handle INT8)
    base_model = AutoModelForCausalLM.from_pretrained(
        model_name,
        config=config,
        torch_dtype=torch.float32,
        low_cpu_mem_usage=True,
    )
    base_model.eval()

    # Wrap model to simplify for SNPE
    model = SimplifiedGemmaWrapper(base_model, seq_length)
    model.eval()

    # Create dummy input (static shape only)
    dummy_input_ids = torch.randint(0, tokenizer.vocab_size, (batch_size, seq_length), dtype=torch.long)

    print(f"\nExporting to ONNX: {output_path}")
    print(f"Input shape: input_ids={dummy_input_ids.shape}")
    print(f"Output shape: logits=({batch_size}, {seq_length}, {config.vocab_size})")

    # Export with static shapes (no dynamic_axes for SNPE)
    torch.onnx.export(
        model,
        (dummy_input_ids,),
        str(output_path),
        input_names=["input_ids"],
        output_names=["logits"],
        # NO dynamic_axes - SNPE requires static shapes
        opset_version=13,  # Use opset 13 for better SNPE compatibility
        do_constant_folding=True,
        verbose=False,
        export_params=True,
    )

    file_size_mb = output_path.stat().st_size / (1024 * 1024)
    print(f"\n✓ ONNX export complete")
    print(f"  File: {output_path}")
    print(f"  Size: {file_size_mb:.1f} MB")

    # Validate ONNX model
    print(f"\nValidating ONNX model...")
    try:
        import onnx
        onnx_model = onnx.load(str(output_path))
        onnx.checker.check_model(onnx_model)
        print(f"  ✓ ONNX model is valid")

        # Print model info
        print(f"\nModel info:")
        print(f"  Opset version: {onnx_model.opset_import[0].version}")
        print(f"  Input: {onnx_model.graph.input[0].name} {[d.dim_value for d in onnx_model.graph.input[0].type.tensor_type.shape.dim]}")
        print(f"  Output: {onnx_model.graph.output[0].name}")

    except ImportError:
        print(f"  ⚠️ onnx package not installed - skipping validation")
        print(f"  Install with: pip install onnx")
    except Exception as e:
        print(f"  ⚠️ Validation failed: {e}")

    print(f"\nNext step: Convert to SNPE DLC format")
    print(f"  export SNPE_ROOT=$PWD/vendor/qairt/2.39.0.250926")
    print(f"  ./scripts/convert-to-snpe.sh {output_path}")

    return output_path


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Export Gemma model to ONNX")
    parser.add_argument(
        "--model",
        default="google/gemma-3-270m-it",
        help="HuggingFace model name or path",
    )
    parser.add_argument(
        "--output",
        help="Output ONNX file path",
    )
    parser.add_argument(
        "--seq-length",
        type=int,
        default=512,
        help="Sequence length for export",
    )
    parser.add_argument(
        "--batch-size",
        type=int,
        default=1,
        help="Batch size for export",
    )

    args = parser.parse_args()

    export_gemma_to_onnx(
        model_name=args.model,
        output_path=args.output,
        seq_length=args.seq_length,
        batch_size=args.batch_size,
    )
