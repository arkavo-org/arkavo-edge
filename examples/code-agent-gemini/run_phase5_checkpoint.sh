#!/bin/bash
set -e

echo "=================================================="
echo "Phase 5 Checkpoint: Cost Orchestrator"
echo "=================================================="
echo

echo "Building arkavo-router with cost orchestrator..."
cargo build -p arkavo-router
echo "✅ Router built successfully"
echo

echo "Building arkavo-agui with cost handler..."
cargo build -p arkavo-agui
echo "✅ AGUI built successfully"
echo

echo "Running arkavo-router tests..."
cargo test -p arkavo-router --lib
echo "✅ Router tests passed"
echo

echo "Running arkavo-agui tests..."
cargo test -p arkavo-agui cost_handler
echo "✅ AGUI cost handler tests passed"
echo

echo "=================================================="
echo "Phase 5 Checkpoint Complete!"
echo "=================================================="
echo
echo "Key deliverables:"
echo "  ✅ CostOrchestrator with budget-aware routing"
echo "  ✅ WorkflowCostPredictor with cost estimation"
echo "  ✅ ROI metrics calculator and dashboard"
echo "  ✅ Cost handler integrated into AGUI"
echo "  ✅ Cost events added to AgUiEvent types"
echo
echo "See benchmarks/phase5_checkpoint.md for detailed results"
