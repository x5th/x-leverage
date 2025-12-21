#!/bin/bash
# Build script wrapper - ensures correct Solana toolchain is used
# This prevents dependency resolution errors from using the wrong rustc version

set -e

# CRITICAL: Use Solana 2.3+ toolchain with rustc 1.84+
export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH"

# Verify correct toolchain
EXPECTED_MAJOR=2
EXPECTED_MINOR=3

echo "🔧 Verifying Solana toolchain..."
TOOLCHAIN_VERSION=$(cargo-build-sbf --version 2>/dev/null | head -n1 || echo "not found")

if [[ "$TOOLCHAIN_VERSION" == *"not found"* ]]; then
    echo "❌ ERROR: cargo-build-sbf not found!"
    echo "   Expected: Solana 2.3+ with rustc 1.84+"
    echo "   Current PATH: $PATH"
    exit 1
fi

echo "   $TOOLCHAIN_VERSION"

# Extract version numbers for validation
if [[ "$TOOLCHAIN_VERSION" =~ ([0-9]+)\.([0-9]+)\. ]]; then
    MAJOR="${BASH_REMATCH[1]}"
    MINOR="${BASH_REMATCH[2]}"

    if [ "$MAJOR" -lt "$EXPECTED_MAJOR" ] || ([ "$MAJOR" -eq "$EXPECTED_MAJOR" ] && [ "$MINOR" -lt "$EXPECTED_MINOR" ]); then
        echo "⚠️  WARNING: Old Solana version detected (found $MAJOR.$MINOR, expected $EXPECTED_MAJOR.$EXPECTED_MINOR+)"
        echo "   This may cause build failures due to dependency resolution issues"
        echo "   Please ensure ~/.local/share/solana-release/bin is in your PATH"
        exit 1
    fi
fi

# Check rustc version
RUSTC_VERSION=$(~/.local/share/solana-release/bin/sdk/sbf/dependencies/platform-tools/rust/bin/rustc --version 2>/dev/null || echo "unknown")
echo "   rustc: $RUSTC_VERSION"

if [[ "$RUSTC_VERSION" == *"1.75"* ]]; then
    echo "❌ ERROR: Detected old rustc 1.75 (from wrong toolchain)"
    echo "   Expected: rustc 1.84+"
    echo "   Fix: Ensure PATH is set correctly in ~/.bashrc"
    exit 1
fi

echo "✅ Toolchain verified!"
echo ""

# Run the build
if [ "$1" == "test" ]; then
    echo "🧪 Running anchor test..."
    exec anchor test "$@"
elif [ "$1" == "clean" ]; then
    echo "🧹 Cleaning build artifacts..."
    exec anchor clean
else
    echo "🔨 Building all programs..."
    exec anchor build "$@"
fi
