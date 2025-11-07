#!/usr/bin/env python3
"""
Download and verify Qwen3-0.6B ONNX model for SNPE conversion.
"""
import argparse
from pathlib import Path
from huggingface_hub import snapshot_download, hf_hub_download


def download_qwen_onnx(
    model_id: str = "onnx-community/Qwen3-0.6B-ONNX",
    output_dir: str = None,
):
    """Download Qwen3-0.6B ONNX model from Hugging Face."""

    if output_dir is None:
        output_dir = Path.home() / ".cache" / "arkavo" / "models" / "qwen3-onnx"

    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    print(f"Downloading {model_id}...")
    print(f"Output directory: {output_dir}")

    try:
        # Download all files from the repository
        local_dir = snapshot_download(
            repo_id=model_id,
            cache_dir=output_dir,
            local_dir=output_dir / "model",
        )

        print(f"\n✓ Download complete: {local_dir}")

        # List downloaded files
        model_path = Path(local_dir)
        onnx_files = list(model_path.glob("*.onnx"))

        if onnx_files:
            print(f"\nFound ONNX files:")
            for f in onnx_files:
                size_mb = f.stat().st_size / (1024 * 1024)
                print(f"  {f.name} ({size_mb:.1f} MB)")
        else:
            print(f"\n⚠️ No .onnx files found in {model_path}")
            print(f"Files present:")
            for f in model_path.iterdir():
                if f.is_file():
                    print(f"  {f.name}")

        # Try to load and inspect ONNX model
        if onnx_files:
            print(f"\nInspecting ONNX model: {onnx_files[0].name}")
            try:
                import onnx
                onnx_model = onnx.load(str(onnx_files[0]))

                print(f"  Opset version: {onnx_model.opset_import[0].version}")
                print(f"\n  Inputs:")
                for inp in onnx_model.graph.input:
                    shape = [d.dim_value if d.dim_value > 0 else d.dim_param for d in inp.type.tensor_type.shape.dim]
                    print(f"    {inp.name}: {shape}")

                print(f"\n  Outputs:")
                for out in onnx_model.graph.output:
                    print(f"    {out.name}")

                # Check for dynamic shapes
                has_dynamic = any(
                    any(d.dim_param for d in inp.type.tensor_type.shape.dim)
                    for inp in onnx_model.graph.input
                )

                if has_dynamic:
                    print(f"\n  ⚠️ Model has dynamic shapes - will need to fix for SNPE")
                else:
                    print(f"\n  ✓ Model has static shapes - SNPE compatible")

            except ImportError:
                print(f"  ⚠️ onnx package not installed - install with: pip install onnx")
            except Exception as e:
                print(f"  ⚠️ Error inspecting ONNX model: {e}")

        print(f"\nNext steps:")
        if onnx_files:
            print(f"  1. If dynamic shapes detected, re-export with static shapes")
            print(f"  2. Convert to SNPE DLC:")
            print(f"     export SNPE_ROOT=$PWD/vendor/qairt/2.39.0.250926")
            print(f"     ./scripts/convert-to-snpe.sh {onnx_files[0]}")
        else:
            print(f"  1. Check repository structure - may need different download method")

        return local_dir

    except Exception as e:
        print(f"\n❌ Download failed: {e}")
        return None


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Download Qwen3-0.6B ONNX model")
    parser.add_argument(
        "--model-id",
        default="onnx-community/Qwen3-0.6B-ONNX",
        help="Hugging Face model ID",
    )
    parser.add_argument(
        "--output",
        help="Output directory",
    )

    args = parser.parse_args()

    download_qwen_onnx(
        model_id=args.model_id,
        output_dir=args.output,
    )
