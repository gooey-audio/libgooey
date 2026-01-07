#!/bin/bash
# Build gooey library for iOS targets
#
# This script cross-compiles the gooey library for iOS devices and simulators.
# Output:
#   - target/aarch64-apple-ios/release/libgooey.a (device)
#   - target/aarch64-apple-ios-sim/release/libgooey.a (simulator)
#   - include/gooey.h (C header)

set -e

cd "$(dirname "$0")/.."

echo "=== Building gooey for iOS ==="
echo ""

# Check if iOS targets are installed
if ! rustup target list --installed | grep -q "aarch64-apple-ios"; then
    echo "Installing iOS target: aarch64-apple-ios"
    rustup target add aarch64-apple-ios
fi

if ! rustup target list --installed | grep -q "aarch64-apple-ios-sim"; then
    echo "Installing iOS simulator target: aarch64-apple-ios-sim"
    rustup target add aarch64-apple-ios-sim
fi

echo ""
echo "Building for iOS device (aarch64-apple-ios)..."
cargo build --release --target aarch64-apple-ios --no-default-features --features ios

echo ""
echo "Building for iOS simulator (aarch64-apple-ios-sim)..."
cargo build --release --target aarch64-apple-ios-sim --no-default-features --features ios

echo ""
echo "=== Build complete ==="
echo ""
echo "Libraries:"
echo "  Device:    target/aarch64-apple-ios/release/libgooey.a"
echo "  Simulator: target/aarch64-apple-ios-sim/release/libgooey.a"
echo ""
echo "Header:"
echo "  include/gooey.h"
echo ""

# Verify the files exist
if [ -f "target/aarch64-apple-ios/release/libgooey.a" ]; then
    echo "Device library size: $(ls -lh target/aarch64-apple-ios/release/libgooey.a | awk '{print $5}')"
else
    echo "WARNING: Device library not found!"
fi

if [ -f "target/aarch64-apple-ios-sim/release/libgooey.a" ]; then
    echo "Simulator library size: $(ls -lh target/aarch64-apple-ios-sim/release/libgooey.a | awk '{print $5}')"
else
    echo "WARNING: Simulator library not found!"
fi
