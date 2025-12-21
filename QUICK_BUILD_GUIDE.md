# Quick Build Guide

## TL;DR - How to Build

```bash
# Just run this - it handles everything automatically:
./build.sh
```

## What's Been Fixed

✅ **Permanent Solution Implemented**
- Your `~/.bashrc` now automatically uses the correct Solana 2.3+ toolchain
- No need to manually export PATH every time
- Build script validates toolchain before building

## Common Commands

```bash
# Build all programs
./build.sh

# Run tests
./build.sh test

# Clean build artifacts
./build.sh clean

# Manual build (if you prefer)
source ~/.bashrc  # Reload shell first!
anchor build
```

## If You See Errors

### "requires rustc 1.76 or newer" or dependency errors
```bash
# This means wrong toolchain is active
# Fix: Reload your shell
source ~/.bashrc
./build.sh
```

### Cargo.lock conflicts
```bash
# Restore the known-good lockfile
git checkout Cargo.lock
./build.sh
```

## For New Terminal Sessions

**Option 1 (Recommended):** Your new terminals automatically have the correct PATH from ~/.bashrc

**Option 2:** Use the build script which forces the correct PATH:
```bash
./build.sh
```

## Verify Your Toolchain

```bash
# Should show: solana-cargo-build-sbf 2.3.13
cargo-build-sbf --version

# Should show rustc 1.84.1 (or newer)
~/.local/share/solana-release/bin/sdk/sbf/dependencies/platform-tools/rust/bin/rustc --version
```

## Why This Matters

This project requires:
- ✅ Solana/Agave 2.3+ (NOT the old 1.18 version)
- ✅ rustc 1.84+ (NOT the old 1.75 version)

Using the wrong toolchain causes:
- ❌ Dependency resolution failures (borsh, toml_edit, etc.)
- ❌ MSRV (Minimum Supported Rust Version) errors
- ❌ Build failures with confusing error messages

The permanent fix ensures you always use the correct versions automatically.
