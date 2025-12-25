#!/bin/bash
set -e

# 1. Compile and run setup
rustc setup_chaos.rs -o setup_chaos
./setup_chaos

# 2. Run Cargo Check to show the noise
echo "--- PRE-CHECK: Generating Noise ---"
cd demo_workspace
if ! cargo check > /dev/null 2>&1; then
    echo "Chaos confirmed: Build failed as expected."
    echo "Sample Error Output:"
    cargo check 2>&1 | head -n 20
    echo "... (thousands of lines omitted) ..."
else
    echo "Error: Chaos failed to break the build!"
    exit 1
fi
cd ..

# 3. Prompt user
echo ""
echo "Now run Arkavo to fix this mess:"
echo "  cargo run -p arkavo -- run --task \"Fix the build errors in demo_workspace caused by the core_lib type change\""
