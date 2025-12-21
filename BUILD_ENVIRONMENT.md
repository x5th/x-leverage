# Build Environment Configuration

## Critical: Use the Correct Solana Toolchain

This project requires **Solana/Agave 2.3+** with **rustc 1.84+** for building.

### The Problem

There are two Solana installations on this system:

1. **Old Installation** (`~/.local/share/solana/install/active_release/`):
   - solana-cli 1.18.26
   - rustc 1.75.0 (TOO OLD)
   - Will fail to compile modern dependencies (borsh 1.6+, etc.)

2. **New Installation** (`~/.local/share/solana-release/`):
   - solana 2.3.13 (Agave)
   - rustc 1.84.1 (REQUIRED)
   - Works with all project dependencies

### ✅ PERMANENT SOLUTION (RECOMMENDED)

The PATH has been permanently configured in `~/.bashrc` to use the correct toolchain.

**Just use the build wrapper script:**

```bash
# Build all programs (verifies toolchain automatically)
./build.sh

# Run tests
./build.sh test

# Clean build artifacts
./build.sh clean
```

The build script will:
- ✅ Automatically use the correct Solana 2.3+ toolchain
- ✅ Verify you have rustc 1.84+ before building
- ✅ Fail fast with clear error messages if wrong toolchain detected
- ✅ Prevent dependency resolution errors

### Manual Build (if needed)

If you need to run `anchor build` directly, **first reload your shell**:

```bash
# Reload shell to pick up correct PATH from ~/.bashrc
source ~/.bashrc

# Verify correct toolchain
cargo-build-sbf --version
# Should show: solana-cargo-build-sbf 2.3.13, platform-tools v1.48, rustc 1.84.1

# Build
anchor build
```

### Troubleshooting

#### Error: "the MSRV of ... is 1.76.0" or "requires rustc 1.77.0 or newer"
**Cause**: Using the old Solana toolchain with rustc 1.75.0
**Solution**:
```bash
# Reload your shell to pick up the correct PATH
source ~/.bashrc

# Or use the build script which handles this automatically
./build.sh
```

#### Error: Cargo.lock version conflicts
**Cause**: Cargo.lock was regenerated with wrong toolchain
**Solution**:
```bash
# Restore the known-good Cargo.lock
git checkout Cargo.lock

# Reload shell and rebuild
source ~/.bashrc
./build.sh
```

#### Build script says "Old Solana version detected"
**Cause**: Your current shell hasn't picked up the updated PATH
**Solution**:
```bash
# Option 1: Reload the shell configuration
source ~/.bashrc

# Option 2: Start a new shell session
exit  # then reconnect
```

#### Error: "idl-build feature is missing"
**Cause**: Program Cargo.toml missing the feature
**Solution**: Add to each program's Cargo.toml:
```toml
[features]
default = []
idl-build = ["anchor-lang/idl-build"]
# If using anchor-spl:
# idl-build = ["anchor-lang/idl-build", "anchor-spl/idl-build"]
```

### Verification

After successful build, you should see 8 program binaries:
```
target/deploy/
  - financing_engine.so
  - governance.so
  - liquidation_engine.so
  - lp_vault.so
  - oracle_framework.so
  - settlement_engine.so
  - treasury_engine.so
  - wrapping_vault.so
```

### ⚠️ IMPORTANT: DO NOT

- ❌ Do not delete or regenerate `Cargo.lock` unless you have the correct toolchain active
- ❌ Do not use `cargo update` without first verifying your toolchain (`cargo-build-sbf --version`)
- ❌ Do not downgrade individual dependencies to work around MSRV errors
- ❌ Do not manually set PATH in individual terminal sessions (it's now permanent in ~/.bashrc)
- ✅ **ALWAYS** use `./build.sh` or reload shell with `source ~/.bashrc` before building

### Last Verified Build

- Date: 2025-12-18
- Solana: 2.3.13 (Agave)
- Rust: 1.84.1
- Anchor: 0.32.0
- All 8 programs built successfully
