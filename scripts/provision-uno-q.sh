#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

usage() {
    cat <<EOF
Provision Arduino UNO Q with Arkavo Edge and SNPE models

Usage: $0 [OPTIONS] <uno_q_host>

Arguments:
    uno_q_host          Hostname or IP address of UNO Q device

Options:
    -u, --user USER     SSH username (default: debian)
    -k, --key PATH      SSH private key path
    -m, --model PATH    DLC model to deploy
    -s, --snpe PATH     SNPE SDK path on host (to copy runtime libs)
    -h, --help          Show this help message

Examples:
    # Basic provisioning with IP address
    $0 192.168.1.100

    # With custom user and model
    $0 --user admin --model models/gemma-3-270m.dlc uno-q.local

    # With SSH key
    $0 --key ~/.ssh/uno-q-key --model models/model.dlc 10.0.0.50

Requirements:
    - SSH access to UNO Q device
    - Arkavo Edge binary built for aarch64-linux
    - SNPE runtime libraries (from SNPE SDK)
    - DLC model file (optional)
EOF
}

UNO_Q_HOST=""
SSH_USER="debian"
SSH_KEY=""
MODEL_PATH=""
SNPE_PATH="${SNPE_ROOT:-}"

while [[ $# -gt 0 ]]; do
    case $1 in
        -u|--user)
            SSH_USER="$2"
            shift 2
            ;;
        -k|--key)
            SSH_KEY="$2"
            shift 2
            ;;
        -m|--model)
            MODEL_PATH="$2"
            shift 2
            ;;
        -s|--snpe)
            SNPE_PATH="$2"
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
            UNO_Q_HOST="$1"
            shift
            ;;
    esac
done

if [ -z "$UNO_Q_HOST" ]; then
    echo "Error: uno_q_host argument required"
    usage
    exit 1
fi

SSH_OPTS="-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"
if [ -n "$SSH_KEY" ]; then
    SSH_OPTS="$SSH_OPTS -i $SSH_KEY"
fi

SSH_CMD="ssh $SSH_OPTS ${SSH_USER}@${UNO_Q_HOST}"
SCP_CMD="scp $SSH_OPTS"

echo "=== UNO Q Provisioning ==="
echo "Host:     $UNO_Q_HOST"
echo "User:     $SSH_USER"
echo "Model:    ${MODEL_PATH:-none}"
echo

echo "Step 1: Verifying SSH connectivity..."
if ! $SSH_CMD "echo 'SSH connection successful'" > /dev/null 2>&1; then
    echo "Error: Cannot connect to $UNO_Q_HOST via SSH"
    echo "Please check:"
    echo "  - Device is powered on and connected to network"
    echo "  - SSH service is running on UNO Q"
    echo "  - Firewall allows SSH connections"
    exit 1
fi
echo "✓ SSH connection verified"

echo
echo "Step 2: Detecting system architecture..."
ARCH=$($SSH_CMD "uname -m")
if [ "$ARCH" != "aarch64" ]; then
    echo "Error: UNO Q architecture mismatch. Expected aarch64, got $ARCH"
    exit 1
fi
echo "✓ Architecture: $ARCH"

echo
echo "Step 3: Creating installation directories..."
$SSH_CMD "sudo mkdir -p /opt/arkavo/{bin,models,logs}" || true
$SSH_CMD "sudo mkdir -p /etc/arkavo" || true
$SSH_CMD "sudo chown -R ${SSH_USER}:${SSH_USER} /opt/arkavo" || true
echo "✓ Directories created"

BINARY_PATH="$PROJECT_ROOT/target/aarch64-unknown-linux-gnu/release/arkavo"
if [ ! -f "$BINARY_PATH" ]; then
    BINARY_PATH="$PROJECT_ROOT/target/release/arkavo"
    if [ ! -f "$BINARY_PATH" ]; then
        echo "Error: Arkavo binary not found"
        echo "Build for aarch64-linux with: cargo build --release --target aarch64-unknown-linux-gnu"
        exit 1
    fi
fi

echo
echo "Step 4: Deploying Arkavo Edge binary..."
$SCP_CMD "$BINARY_PATH" "${SSH_USER}@${UNO_Q_HOST}:/opt/arkavo/bin/"
$SSH_CMD "chmod +x /opt/arkavo/bin/arkavo"
VERSION=$($SSH_CMD "/opt/arkavo/bin/arkavo --version" || echo "unknown")
echo "✓ Deployed Arkavo Edge: $VERSION"

if [ -n "$SNPE_PATH" ]; then
    echo
    echo "Step 5: Deploying SNPE runtime libraries..."
    SNPE_LIBS="$SNPE_PATH/lib/aarch64-linux"
    if [ ! -d "$SNPE_LIBS" ]; then
        echo "Warning: SNPE libraries not found at $SNPE_LIBS"
    else
        $SSH_CMD "sudo mkdir -p /opt/snpe/lib" || true
        $SCP_CMD -r "$SNPE_LIBS" "${SSH_USER}@${UNO_Q_HOST}:/opt/snpe/lib/"
        echo "✓ SNPE runtime deployed to /opt/snpe"
    fi
fi

if [ -n "$MODEL_PATH" ] && [ -f "$MODEL_PATH" ]; then
    echo
    echo "Step 6: Deploying DLC model..."
    MODEL_NAME=$(basename "$MODEL_PATH")
    $SCP_CMD "$MODEL_PATH" "${SSH_USER}@${UNO_Q_HOST}:/opt/arkavo/models/"
    echo "✓ Model deployed: $MODEL_NAME"
fi

echo
echo "Step 7: Creating environment configuration..."
cat > /tmp/arkavo-env <<'EOF'
export SNPE_ROOT=/opt/snpe
export LD_LIBRARY_PATH=$SNPE_ROOT/lib/aarch64-linux:${LD_LIBRARY_PATH:-}
export ARKAVO_SNPE_RUNTIME=GPU_FP16
export ARKAVO_MODEL_PATH=/opt/arkavo/models
export ARKAVO_DEBUG=1
EOF

$SCP_CMD /tmp/arkavo-env "${SSH_USER}@${UNO_Q_HOST}:/opt/arkavo/"
$SSH_CMD "cat /opt/arkavo/arkavo-env >> ~/.bashrc" || true
rm /tmp/arkavo-env
echo "✓ Environment configured"

echo
echo "Step 8: Creating systemd service..."
cat > /tmp/arkavo.service <<EOF
[Unit]
Description=Arkavo Edge Agent with SNPE
After=network.target

[Service]
Type=simple
User=${SSH_USER}
WorkingDirectory=/opt/arkavo
EnvironmentFile=/opt/arkavo/arkavo-env
ExecStart=/opt/arkavo/bin/arkavo
Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF

$SCP_CMD /tmp/arkavo.service "${SSH_USER}@${UNO_Q_HOST}:/tmp/"
$SSH_CMD "sudo mv /tmp/arkavo.service /etc/systemd/system/"
$SSH_CMD "sudo systemctl daemon-reload"
rm /tmp/arkavo.service
echo "✓ Systemd service created"

echo
echo "Step 9: Running validation tests..."
if $SSH_CMD "source /opt/arkavo/arkavo-env && /opt/arkavo/bin/arkavo --version" > /dev/null 2>&1; then
    echo "✓ Binary execution validated"
else
    echo "Warning: Binary validation failed"
fi

echo
echo "=== Provisioning Complete ==="
echo
echo "Service Management:"
echo "  Start:  ssh ${SSH_USER}@${UNO_Q_HOST} 'sudo systemctl start arkavo'"
echo "  Stop:   ssh ${SSH_USER}@${UNO_Q_HOST} 'sudo systemctl stop arkavo'"
echo "  Status: ssh ${SSH_USER}@${UNO_Q_HOST} 'sudo systemctl status arkavo'"
echo "  Logs:   ssh ${SSH_USER}@${UNO_Q_HOST} 'journalctl -u arkavo -f'"
echo
echo "Enable auto-start on boot:"
echo "  ssh ${SSH_USER}@${UNO_Q_HOST} 'sudo systemctl enable arkavo'"
echo
echo "Manual testing:"
echo "  ssh ${SSH_USER}@${UNO_Q_HOST}"
echo "  source /opt/arkavo/arkavo-env"
echo "  /opt/arkavo/bin/arkavo chat --model snpe --prompt 'test'"
