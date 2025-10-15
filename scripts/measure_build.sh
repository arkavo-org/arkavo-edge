#!/bin/bash
set -e

echo "--- Running Clean Release Build ---"

# Clean the project to ensure a full rebuild
cargo clean

# Run the build and capture the output of the `time` command
TIME_OUTPUT=$( (time cargo build --release --timings) 2>&1 )
echo "$TIME_OUTPUT"

# Extract the total build time from the `time` command's output
TOTAL_TIME=$(echo "$TIME_OUTPUT" | grep 'real' | awk '{print $2}')

# Find the latest timings report HTML file
TIMINGS_FILE=$(find target/cargo-timings -name "*.html" -print0 | xargs -0 ls -t | head -n 1)

# Extract the link time for the 'arkavo' binary from the HTML report.
# This is more robust: it finds the line with the 'arkavo' data, then extracts the 'link' value specifically.
LINK_TIME_SECONDS=$(grep '"name":"arkavo"' "$TIMINGS_FILE" | sed -n 's/.*"link":\([0-9.]*\).*/\1/p')

echo "---"
echo "Build Measurement Complete:"
echo "Total Build Time: ${TOTAL_TIME}"
echo "Final Link Time: ${LINK_TIME_SECONDS}s"
echo "---"
