# How This Ensures Claude Code Always Uses Correct Toolchain

## The Challenge

Claude Code needs to remember across sessions to always use:
- Solana 2.3+ (NOT 1.18)
- rustc 1.84+ (NOT 1.75)

Without this, build failures occur due to dependency resolution issues.

## The Solution: Multi-Layered Approach

### 1. Project-Level Configuration Files (This Directory)

**`.claude/INSTRUCTIONS_FOR_CLAUDE.txt`**
- Concise, critical instructions at the top of file
- Quick reference for the correct build command
- First thing Claude should check when building

**`.claude/build-config.md`**
- Comprehensive build configuration documentation
- Examples of correct vs incorrect commands
- Troubleshooting guide for common errors
- Quick reference table

**`.claude/README.md`**
- Directory overview
- Tells Claude to read build-config.md

### 2. Shell Environment (For Manual/Direct Usage)

**`~/.bashrc` and `~/.profile`**
- Permanently configured with correct Solana 2.3+ path
- New shell sessions automatically get correct toolchain
- Prevents accidental use of old toolchain

### 3. Build Wrapper Script (Foolproof Method)

**`build.sh`**
- Forces correct PATH regardless of shell state
- Validates toolchain before building
- Fails fast with clear error messages
- Recommended method for all builds

### 4. Documentation (Human Reference)

**`BUILD_ENVIRONMENT.md`**
- Comprehensive guide for both Claude and humans
- Explains the problem and solution
- Troubleshooting section

**`QUICK_BUILD_GUIDE.md`**
- TL;DR quick reference
- Common commands
- Quick troubleshooting

**`TOOLCHAIN_FIX_SUMMARY.md`**
- What was changed and why
- Before/after comparison
- Verification steps

## How Claude Code Should Use This

### On Project Start / New Session

1. **Check for `.claude/` directory** - If present, read the instructions
2. **Read `.claude/INSTRUCTIONS_FOR_CLAUDE.txt`** - Get critical build requirements
3. **For detailed info**: Read `.claude/build-config.md`

### When Building

**Priority 1 (Recommended):**
```bash
./build.sh
```
- Most foolproof
- Always uses correct toolchain
- Validates before building

**Priority 2 (If build.sh not available):**
```bash
export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH" && anchor build
```
- Explicitly sets correct toolchain
- Works in any shell state

**❌ Never Do This:**
```bash
anchor build  # May use wrong toolchain!
```

### When Seeing Dependency Errors

1. **DON'T** try to fix with `cargo update`
2. **DO** restore Cargo.lock: `git checkout Cargo.lock`
3. **DO** rebuild with correct toolchain (see above)

## Why This Works

### For Claude Code:
- ✅ Visible project configuration in `.claude/` directory
- ✅ Clear, searchable instructions
- ✅ Examples of correct commands to copy
- ✅ Troubleshooting guide for when things go wrong

### For Humans:
- ✅ Shell environment permanently configured
- ✅ Build script for convenience
- ✅ Documentation for understanding

### For Both:
- ✅ Multiple layers of protection
- ✅ Clear error messages when wrong toolchain used
- ✅ Easy to verify correct configuration
- ✅ Git-tracked configuration (survives across environments)

## Verification

Claude Code can verify correct setup by checking:

```bash
# 1. Check if .claude/ directory exists
ls -la .claude/

# 2. Check if build script exists
ls -la build.sh

# 3. Verify toolchain
export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH" && cargo-build-sbf --version
# Should show: solana-cargo-build-sbf 2.3.13
```

## What Makes This Different

**Previous attempts:**
- Manual PATH exports (easy to forget)
- Documentation only (not always read)
- Shell config only (not always active in current session)

**This solution:**
- ✅ Project-specific `.claude/` directory (hard to miss)
- ✅ Multiple reminder mechanisms
- ✅ Automated validation (build.sh)
- ✅ Git-tracked (persists across sessions/machines)
- ✅ Clear priority: Use build.sh OR export PATH

## Success Criteria

Claude Code is successfully using correct toolchain when:
1. ✅ Builds succeed without dependency errors
2. ✅ No MSRV (Minimum Supported Rust Version) errors
3. ✅ No Cargo.lock conflicts
4. ✅ Consistent build results

If any of the above fail, Claude should:
1. Check `.claude/build-config.md`
2. Verify using correct build command
3. Restore Cargo.lock if needed
4. Try again with correct toolchain

---

**Created:** 2025-12-21
**Purpose:** Ensure Claude Code always uses correct Solana toolchain across all sessions
**Status:** Active - Reference this file when building
