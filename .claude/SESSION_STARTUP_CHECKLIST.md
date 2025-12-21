# Claude Code Session Startup Checklist

## 🚀 Start of Every Session

When Claude Code starts a new session on this project, perform these checks:

### 1. Verify GitHub Authentication ✅

```bash
gh auth status
```

**Expected output:**
```
✓ Logged in to github.com account x5th
- Token: ghp_************************************
```

**If NOT authenticated:**
```bash
# Restore from backup (easiest)
bash /root/GITHUB_ACCESS_RECOVERY.sh

# Or manual restore:
cat /root/.github_token_backup | gh auth login --with-token
gh auth setup-git
```

### 2. Verify Build Toolchain ✅

```bash
cargo-build-sbf --version
```

**Expected output:**
```
solana-cargo-build-sbf 2.3.13
platform-tools v1.48
rustc 1.84.1
```

**If wrong version:**
```bash
source ~/.bashrc
# Or use build script:
./build.sh
```

### 3. Check Git Status ✅

```bash
git status
git branch
```

**Verify:**
- On correct branch
- No unexpected changes
- Remote is set correctly

### 4. Review Recent Work ✅

```bash
git log --oneline -5
gh pr list
```

**Check:**
- Last commits make sense
- Any open PRs
- Current state of project

## 📋 Quick Commands Reference

### GitHub Authentication
```bash
gh auth status              # Check if authenticated
gh-check                   # Alias for above
gh-restore                 # Restore authentication if lost
```

### Git Operations
```bash
git push origin <branch>   # Push branch
gh pr create --fill        # Create PR from commits
gh pr list                 # View open PRs
```

### Building
```bash
./build.sh                 # Build all programs (recommended)
./build.sh test           # Run tests
```

## 🔐 Token Recovery Locations

If authentication is lost, token can be found in:

1. **Primary backup:** `/root/x-leverage/.claude/CRITICAL_GITHUB_ACCESS.md`
2. **Secondary backup:** `/root/.github_token_backup`
3. **Recovery script:** `/root/GITHUB_ACCESS_RECOVERY.sh`

## ⚠️ Critical Files - Never Delete

- `~/.config/gh/hosts.yml` - gh CLI authentication
- `.claude/CRITICAL_GITHUB_ACCESS.md` - Token backup
- `/root/.github_token_backup` - Token plain text backup
- `/root/GITHUB_ACCESS_RECOVERY.sh` - Auto-recovery script

## 🎯 Session Goals Template

At start of each session, determine:

1. **What needs to be done?** (Check existing todos, PRs, issues)
2. **What was last worked on?** (git log, recent commits)
3. **Are there blockers?** (Failed builds, auth issues, etc.)
4. **What's the priority?** (User request, or continue previous work)

## ✅ Pre-Flight Checklist

Before starting any work:

- [ ] GitHub authenticated (`gh auth status`)
- [ ] Correct toolchain (`cargo-build-sbf --version`)
- [ ] On correct branch (`git branch`)
- [ ] Latest changes pulled if needed (`git pull`)
- [ ] Build environment working (`./build.sh` succeeds)

## 🆘 Emergency Recovery

If everything is broken:

```bash
# 1. Restore GitHub access
bash /root/GITHUB_ACCESS_RECOVERY.sh

# 2. Fix build toolchain
source ~/.bashrc
export PATH="$HOME/.local/share/solana-release/bin:$HOME/.cargo/bin:$PATH"

# 3. Restore dependencies
git checkout Cargo.lock

# 4. Test build
./build.sh

# 5. Verify git
git status
gh pr list
```

---

**Last Updated:** 2025-12-21
**Purpose:** Ensure smooth session startup and prevent loss of critical access/configuration
