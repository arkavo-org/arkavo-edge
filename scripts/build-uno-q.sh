#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "=== Arkavo Edge Build for UNO Q ==="
echo

# Check if running on aarch64 Linux
ARCH=$(uname -m)
OS=$(uname -s)

if [ "$ARCH" != "aarch64" ]; then
    echo "⚠️  Warning: Not running on aarch64 architecture (detected: $ARCH)"
    echo "This script is intended for native builds on Arduino UNO Q"
    echo
    read -p "Continue anyway? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

if [ "$OS" != "Linux" ]; then
    echo "Error: This script requires Linux (detected: $OS)"
    exit 1
fi

# Check for SNPE SDK
if [ -z "${SNPE_ROOT:-}" ]; then
    echo "⚠️  SNPE_ROOT environment variable not set"
    echo
    echo "To enable SNPE hardware acceleration, set SNPE_ROOT:"
    echo "  export SNPE_ROOT=/opt/snpe"
    echo
    read -p "Build without SNPE support? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Exiting. Please set SNPE_ROOT and try again."
        exit 1
    fi
    BUILD_FEATURES=""
    echo "Building without SNPE support (CPU inference only)"
else
    echo "✓ SNPE_ROOT detected: $SNPE_ROOT"

    # Verify SNPE installation
    if [ ! -d "$SNPE_ROOT/lib/aarch64-linux" ]; then
        echo "Error: SNPE libraries not found at $SNPE_ROOT/lib/aarch64-linux"
        exit 1
    fi

    if [ ! -f "$SNPE_ROOT/lib/aarch64-linux/libSNPE.so" ]; then
        echo "Error: libSNPE.so not found at $SNPE_ROOT/lib/aarch64-linux/"
        exit 1
    fi

    echo "✓ SNPE SDK libraries found"
    BUILD_FEATURES="snpe"
    echo "Building with SNPE hardware acceleration support"
fi

echo

# Check for Rust
if ! command -v cargo &> /dev/null; then
    echo "Error: Rust/Cargo not found"
    echo
    echo "Install Rust with:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo "  source \$HOME/.cargo/env"
    exit 1
fi

echo "Rust version:"
rustc --version
cargo --version
echo

# Check for build dependencies
echo "Checking build dependencies..."
MISSING_DEPS=()

if ! command -v cmake &> /dev/null; then
    MISSING_DEPS+=("cmake")
fi

if ! command -v clang &> /dev/null; then
    MISSING_DEPS+=("clang")
fi

if ! command -v pkg-config &> /dev/null; then
    MISSING_DEPS+=("pkg-config")
fi

if [ ${#MISSING_DEPS[@]} -gt 0 ]; then
    echo "Error: Missing required dependencies: ${MISSING_DEPS[*]}"
    echo
    echo "Install with:"
    echo "  sudo apt install -y build-essential cmake clang libclang-dev pkg-config"
    exit 1
fi

echo "✓ All build dependencies found"
echo

# Build
cd "$PROJECT_ROOT"

echo "Building Arkavo Edge..."
echo "This may take 10-20 minutes on first build..."
echo

if [ -n "$BUILD_FEATURES" ]; then
    cargo build --release -p arkavo --features "$BUILD_FEATURES"
else
    cargo build --release -p arkavo
fi

echo
echo "=== Build Complete ==="
echo

# Check binary
BINARY="$PROJECT_ROOT/target/release/arkavo"
if [ ! -f "$BINARY" ]; then
    echo "Error: Binary not found at $BINARY"
    exit 1
fi

SIZE=$(stat --format=%s "$BINARY" 2>/dev/null || stat -f%z "$BINARY")
SIZE_MB=$((SIZE / 1024 / 1024))

echo "Binary: $BINARY"
echo "Size:   ${SIZE_MB}MB"
echo

# Check if SNPE is linked
if [ -n "$BUILD_FEATURES" ]; then
    echo "Checking SNPE linkage..."
    if ldd "$BINARY" | grep -q SNPE; then
        echo "✓ SNPE libraries linked successfully"
    else
        echo "⚠️  SNPE libraries not found in binary"
        echo "Binary may not have SNPE support enabled"
    fi
    echo
fi

# Installation prompt
echo "Install to /usr/local/bin? (requires sudo)"
read -p "Install? (y/N) " -n 1 -r
echo

if [[ $REPLY =~ ^[Yy]$ ]]; then
    sudo cp "$BINARY" /usr/local/bin/arkavo
    sudo chmod +x /usr/local/bin/arkavo
    echo "✓ Installed to /usr/local/bin/arkavo"

    # Environment setup
    if [ -n "$BUILD_FEATURES" ]; then
        echo
        echo "Setting up environment for SNPE..."

        ENV_FILE="$HOME/.arkavo-uno-q-env"
        cat > "$ENV_FILE" <<EOF
# Arkavo Edge environment for UNO Q with SNPE
export SNPE_ROOT=$SNPE_ROOT
export LD_LIBRARY_PATH=\$SNPE_ROOT/lib/aarch64-linux:\${LD_LIBRARY_PATH:-}
export ARKAVO_SNPE_RUNTIME=GPU_FP16
export ARKAVO_DEBUG=1
EOF

        echo "Created environment file: $ENV_FILE"
        echo
        echo "Add to your shell profile:"
        echo "  echo 'source $ENV_FILE' >> ~/.bashrc"
        echo "  source ~/.bashrc"
        echo
        echo "Or source manually:"
        echo "  source $ENV_FILE"
    fi

    echo
    echo "Verify installation:"
    echo "  arkavo --version"
else
    echo
    echo "Binary location: $BINARY"
    echo
    echo "To install manually:"
    echo "  sudo cp $BINARY /usr/local/bin/arkavo"
fi

echo
echo "=== Build Summary ==="
echo "Architecture: $ARCH"
echo "SNPE Support: $([ -n "$BUILD_FEATURES" ] && echo "Yes" || echo "No")"
echo "Binary Size:  ${SIZE_MB}MB"
echo
echo "Next steps:"
if [ -n "$BUILD_FEATURES" ]; then
    echo "1. Set up environment (see above)"
    echo "2. Deploy a DLC model to /opt/arkavo/models/"
    echo "3. Run: arkavo chat --model snpe --prompt 'test'"
else
    echo "1. Run: arkavo --version"
    echo "2. For SNPE support, rebuild with SNPE_ROOT set"
fi
