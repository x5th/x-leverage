# Claude Code Build Configuration

## ⚠️ CRITICAL: Always Use This Build Method

**For ALL builds in this project, Claude Code MUST use:**

```bash
# CORRECT - Always use this:
export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH" && anchor build

# EVEN BETTER - Use the build script:
./build.sh
```

## Why This Matters

This project requires:
- ✅ Solana 2.3.13+ (Agave)
- ✅ rustc 1.84.1+
- ❌ NOT Solana 1.18 (old, rustc 1.75)

### The Problem

There are TWO Solana installations on this system:
1. **OLD (WRONG)**: `~/.local/share/solana/install/active_release/` - rustc 1.75 ❌
2. **NEW (CORRECT)**: `~/.local/share/solana-release/` - rustc 1.84 ✅

Using the wrong toolchain causes:
- Dependency resolution failures (borsh, toml_edit, etc.)
- MSRV errors ("requires rustc 1.76 or newer")
- Build failures with confusing messages

## Correct Build Commands for Claude Code

### Building Programs

```bash
# ALWAYS prepend the correct PATH:
export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH" && anchor build

# Or use the wrapper script (preferred):
./build.sh
```

### Running Tests

```bash
export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH" && anchor test
# or
./build.sh test
```

### Individual Program Builds

```bash
export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH" && timeout 300 ~/.local/share/solana-release/bin/cargo-build-sbf --manifest-path programs/PROGRAM_NAME/Cargo.toml
```

## If Build Fails with Dependency Errors

### WRONG Approach (Don't Do This):
```bash
# ❌ Don't try to fix dependencies with cargo update
cargo update toml_edit --precise X.X.X

# ❌ Don't regenerate Cargo.lock
cargo generate-lockfile
```

### CORRECT Approach:
```bash
# ✅ Restore Cargo.lock and use correct toolchain
git checkout Cargo.lock
export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH" && anchor build
```

## Verification Commands

Before building, verify correct toolchain:

```bash
export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH" && cargo-build-sbf --version
# Should show: solana-cargo-build-sbf 2.3.13
```

## Quick Reference

| Task | Command |
|------|---------|
| Build all programs | `export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH" && anchor build` |
| Use build script | `./build.sh` |
| Run tests | `export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH" && anchor test` |
| Build one program | `export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH" && cargo-build-sbf --manifest-path programs/NAME/Cargo.toml` |
| Verify toolchain | `export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH" && cargo-build-sbf --version` |
| Fix broken build | `git checkout Cargo.lock` then build with correct PATH |

## Remember

- **ALWAYS** export the correct PATH before any cargo or anchor commands
- **NEVER** use bare `anchor build` without setting PATH first
- **PREFER** using `./build.sh` which handles PATH automatically
- **IF** you see MSRV errors, it means wrong toolchain was used

---
**Last Updated:** 2025-12-21
**Status:** Active project requirement - DO NOT IGNORE
