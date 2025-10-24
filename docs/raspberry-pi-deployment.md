# Raspberry Pi 5 Deployment Guide

This guide covers deploying Arkavo Edge on Raspberry Pi 5 (16GB) with the gemma-3 270M model for local AI agent orchestration.

## Hardware Requirements

**Minimum Configuration:**
- **Board**: Raspberry Pi 5 (16GB RAM model)
- **Storage**: 32GB+ microSD card or NVMe SSD (recommended for better I/O performance)
- **Cooling**: Active cooling (fan or heatsink) required for sustained inference
- **Power**: Official 27W USB-C power supply

**Performance Expectations:**
- **Inference speed**: 5-15 tokens/second (gemma-3 270M Q4_0 model)
- **Memory usage**: ~2-3 GB (out of 16GB available)
- **Model load time**: <30 seconds
- **Thermal**: CPU may throttle under sustained load without active cooling

## Operating System Setup

Install Raspberry Pi OS (64-bit, Debian 12 Bookworm):

```bash
# Use Raspberry Pi Imager or flash manually
# Download: https://www.raspberrypi.com/software/

# Verify ARM64 architecture
uname -m  # Should output: aarch64

# Update system
sudo apt update && sudo apt upgrade -y
```

## Installation

### Option 1: Pre-built Binary (Recommended)

Download the ARM64 Linux binary from the [releases page](https://github.com/arkavo-org/arkavo-edge/releases):

```bash
# Download latest ARM64 build
wget https://github.com/arkavo-org/arkavo-edge/releases/latest/download/arkavo-aarch64-linux.tar.gz

# Extract
tar -xzf arkavo-aarch64-linux.tar.gz

# Install to system
sudo cp arkavo-aarch64-linux /usr/local/bin/arkavo
sudo chmod +x /usr/local/bin/arkavo

# Verify installation
arkavo --version
```

### Option 2: Build from Source

Building natively on Raspberry Pi 5 (takes ~20-30 minutes):

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Install build dependencies
sudo apt install -y cmake clang libclang-dev build-essential

# Clone repository
git clone --recursive https://github.com/arkavo-org/arkavo-edge.git
cd arkavo-edge

# Build (use debug for faster compilation during development)
cargo build --release -p arkavo

# Install
sudo cp target/release/arkavo /usr/local/bin/
```

## Model Download

Download the gemma-3 270M Q4_0 quantized model:

```bash
# Create model directory
mkdir -p ~/.cache/arkavo/models

# Download from Hugging Face (requires huggingface-cli or wget)
# Using bartowski's GGUF quantizations
wget https://huggingface.co/bartowski/google_gemma-3-270m-it-GGUF/resolve/main/gemma-3-270m-it-qat-Q4_0.gguf \
  -O ~/.cache/arkavo/models/gemma-3-270m-it-Q4_0.gguf

# Verify download (should be ~150-170 MB)
ls -lh ~/.cache/arkavo/models/
```

## Configuration

Enable Raspberry Pi optimizations:

```bash
# Create environment file
cat > ~/.arkavo-env <<EOF
# Enable Raspberry Pi optimizations
export ARKAVO_RASPBERRY_PI=1

# Enable debug logging for llama.cpp (optional)
export ARKAVO_DEBUG=1

# Model path
export ARKAVO_MODEL_PATH=~/.cache/arkavo/models/gemma-3-270m-it-Q4_0.gguf
EOF

# Load environment
source ~/.arkavo-env
```

**What the optimizations do:**
- Reduces context window from 32K to 2048 tokens (faster inference)
- Reduces batch size from 2048 to 512 (lower memory pressure)
- Reduces micro-batch from 512 to 256 (better responsiveness)
- Auto-detects 4-core CPU and optimizes threading

## Running Arkavo

### Interactive Agent

```bash
# Source environment
source ~/.arkavo-env

# Launch agent with auto-configuration
arkavo

# Or specify model explicitly
arkavo --model ~/.cache/arkavo/models/gemma-3-270m-it-Q4_0.gguf
```

### Systemd Service (Run on Boot)

Create a systemd service for automatic startup:

```bash
# Create service file
sudo tee /etc/systemd/system/arkavo.service > /dev/null <<EOF
[Unit]
Description=Arkavo Edge Agent
After=network.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi
Environment="ARKAVO_RASPBERRY_PI=1"
Environment="ARKAVO_MODEL_PATH=/home/pi/.cache/arkavo/models/gemma-3-270m-it-Q4_0.gguf"
ExecStart=/usr/local/bin/arkavo
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF

# Enable and start service
sudo systemctl daemon-reload
sudo systemctl enable arkavo
sudo systemctl start arkavo

# Check status
sudo systemctl status arkavo

# View logs
sudo journalctl -u arkavo -f
```

## Performance Tuning

### CPU Governor

Set CPU governor to performance mode for consistent inference speed:

```bash
# Check current governor
cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor

# Set to performance (temporary)
echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor

# Make permanent (add to /etc/rc.local or systemd service)
sudo tee /etc/systemd/system/cpu-performance.service > /dev/null <<EOF
[Unit]
Description=Set CPU governor to performance

[Service]
Type=oneshot
ExecStart=/bin/sh -c "echo performance | tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor"

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl enable cpu-performance
```

### Cooling

Monitor CPU temperature and throttling:

```bash
# Install monitoring tools
sudo apt install -y lm-sensors

# Monitor temperature in real-time
watch -n 1 vcgencmd measure_temp

# Check for throttling
vcgencmd get_throttled
# 0x0 = No throttling
# Non-zero = Throttling occurred
```

**Recommendations:**
- Use active cooling (fan) for sustained workloads
- Keep CPU temperature below 70°C for optimal performance
- Consider a heatsink case or external fan

### Storage Performance

For best model loading performance, use NVMe SSD:

```bash
# If using NVMe, move models to SSD
sudo mkdir -p /mnt/nvme/arkavo/models
sudo chown pi:pi /mnt/nvme/arkavo/models
mv ~/.cache/arkavo/models/* /mnt/nvme/arkavo/models/
ln -s /mnt/nvme/arkavo/models ~/.cache/arkavo/models
```

## Multi-Agent Mesh

Raspberry Pi 5 can participate in Arkavo mesh networks via mDNS:

```bash
# Agents auto-discover each other on local network
# No configuration needed - mDNS is enabled by default

# Check discovered agents
arkavo agents list

# Configure agent name (optional)
export ARKAVO_AGENT_NAME="pi5-workshop"
arkavo
```

## Troubleshooting

### Out of Memory

If you encounter OOM errors:

```bash
# Reduce context further
export ARKAVO_RASPBERRY_PI=1  # Already sets n_ctx=2048

# Use Q4_0 quantization (smallest usable)
# Q2_K is too aggressive and hurts quality
```

### Slow Inference

If inference is slower than expected:

```bash
# Check CPU throttling
vcgencmd get_throttled

# Verify CPU frequency
watch -n 1 cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq

# Enable performance mode
echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor

# Check debug output
export ARKAVO_DEBUG=1
arkavo chat --prompt "test"
# Look for: Context[Low-Power/Pi]: cores=4, threads=4, n_ctx=2048, n_batch=512
```

### Build Failures

If building from source fails:

```bash
# Ensure all dependencies installed
sudo apt install -y cmake clang libclang-dev build-essential pkg-config

# Increase swap for compilation (if needed)
sudo dphys-swapfile swapoff
sudo nano /etc/dphys-swapfile
# Set CONF_SWAPSIZE=2048
sudo dphys-swapfile setup
sudo dphys-swapfile swapon

# Try debug build (faster compilation)
cargo build -p arkavo
```

## Benchmarking

Test your Raspberry Pi 5 performance:

```bash
# Simple benchmark
source ~/.arkavo-env
time arkavo chat --prompt "Write a haiku about computing" --max-tokens 50

# Expected: ~3-10 seconds for 50 tokens (5-15 t/s)

# Sustained load test
arkavo chat --prompt "Write a comprehensive guide to Rust programming" --max-tokens 500

# Monitor during test
watch -n 1 "vcgencmd measure_temp && cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq"
```

## Monitoring

Track agent performance:

```bash
# Install monitoring tools
sudo apt install -y htop

# Monitor in real-time
htop

# Check memory usage
free -h

# Disk usage
df -h
```

## Security

Secure your Raspberry Pi deployment:

```bash
# Change default password
passwd

# Enable firewall
sudo apt install -y ufw
sudo ufw allow ssh
sudo ufw allow 8080/tcp  # Arkavo UI port (adjust as needed)
sudo ufw enable

# Auto-update security patches
sudo apt install -y unattended-upgrades
sudo dpkg-reconfigure --priority=low unattended-upgrades
```

## Next Steps

- **Explore agent orchestration**: Use `arkavo ui` for visual mesh management
- **Test remote LLM fallback**: Configure cloud models for complex queries
- **Join the community**: Share your Pi 5 performance results and optimizations

## References

- [Raspberry Pi 5 specifications](https://www.raspberrypi.com/products/raspberry-pi-5/)
- [llama.cpp on ARM](https://github.com/ggml-org/llama.cpp)
- [Gemma-3 270M model](https://huggingface.co/google/gemma-3-270m)
- [Arkavo Edge documentation](https://github.com/arkavo-org/arkavo-edge)
