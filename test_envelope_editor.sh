#!/bin/bash
# Test script for envelope_editor example
# Run this from your actual terminal (not through the bash tool)

echo "Building envelope_editor example..."
cargo build --example envelope_editor --features native,visualization,crossterm

if [ $? -eq 0 ]; then
    echo ""
    echo "Build successful! Starting envelope editor..."
    echo ""
    ./target/debug/examples/envelope_editor
else
    echo "Build failed!"
    exit 1
fi
