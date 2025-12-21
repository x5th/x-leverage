# 🔐 CRITICAL: GitHub Access Configuration

## ⚠️ IMPORTANT: This file contains critical authentication information

### GitHub Personal Access Token

**Token Location:** `/root/.github_token_backup` (LOCAL ONLY - NOT IN GIT)

**Account:** x5th
**Repository:** https://github.com/x5th/x-leverage

**To retrieve token:**
```bash
cat /root/.github_token_backup
```

### Token Storage Locations

1. **Primary:** `~/.config/gh/hosts.yml` (gh CLI configuration)
2. **Backup:** This file (for recovery if needed)

### How to Verify Access

```bash
# Check if gh is authenticated
gh auth status

# Should show:
# ✓ Logged in to github.com account x5th
# - Token: ghp_************************************
```

### How to Restore Access (If Lost)

If authentication is ever lost, run:

```bash
# Method 1: Using gh CLI and backup file
cat /root/.github_token_backup | gh auth login --with-token
gh auth setup-git

# Method 2: Using recovery script
bash /root/GITHUB_ACCESS_RECOVERY.sh

# Method 3: Manual configuration
# Token is stored in /root/.github_token_backup
# Copy to ~/.config/gh/hosts.yml in the format shown in github-auth-config.md
```

### Verification Commands

```bash
# Test authentication
gh auth status

# Test git operations
git push --dry-run

# List repositories (should work without prompting)
gh repo list x5th
```

### Token Scopes

This token has FULL access including:
- ✅ repo (full control)
- ✅ workflow
- ✅ admin:org
- ✅ admin:repo_hook
- ✅ delete:packages
- ✅ write:packages
- ✅ All other comprehensive scopes

### Git Configuration

Git credential helper is configured to use gh:

```bash
git config --global credential.helper
# Returns: (empty or shows gh integration)

# If not configured, run:
gh auth setup-git
```

### Common Operations

**Push to GitHub:**
```bash
git push origin <branch-name>
# No credentials needed
```

**Create Pull Request:**
```bash
gh pr create --base main --head <branch> --title "Title" --body "Description"
```

**List PRs:**
```bash
gh pr list
gh pr view <number>
```

### Troubleshooting

#### "Authentication failed"
```bash
# Re-authenticate with saved token
cat /root/.github_token_backup | gh auth login --with-token
gh auth setup-git
```

#### "Token not found" or "Permission denied"
```bash
# Restore from backup file
bash /root/GITHUB_ACCESS_RECOVERY.sh
```

#### "Could not read Username"
```bash
# Ensure gh is setup as git credential helper
gh auth setup-git
git config --global --list | grep credential
```

### For Claude Code Sessions

In any future session, Claude Code should:

1. **First, check authentication:**
   ```bash
   gh auth status
   ```

2. **If not authenticated, restore from backup:**
   ```bash
   cat /root/.github_token_backup | gh auth login --with-token
   gh auth setup-git
   ```

3. **Verify it works:**
   ```bash
   gh repo view x5th/x-leverage
   ```

### Security Notes

- ✅ Token is stored in `~/.config/gh/hosts.yml` with restricted permissions (700)
- ✅ This backup file is in `.claude/` directory (tracked in git for persistence)
- ✅ Token never expires unless manually revoked on GitHub
- ⚠️ Do NOT share this token publicly
- ⚠️ Do NOT commit to public repositories outside this project

### Recovery Procedure

If you ever lose access and need to recover:

1. Check if token file exists: `cat ~/.config/gh/hosts.yml`
2. If missing, read token from this file
3. Re-authenticate: `echo "TOKEN" | gh auth login --with-token`
4. Setup git: `gh auth setup-git`
5. Test: `gh auth status`

### Token Management

**View on GitHub:** https://github.com/settings/tokens

**Token Created:** Before 2025-12-21
**Token Scopes:** Full access (repo, workflow, admin, etc.)
**Token Expiration:** No expiration (unless manually revoked)

### Quick Recovery Commands

```bash
# One-liner to restore everything:
cat /root/.github_token_backup | gh auth login --with-token && gh auth setup-git && gh auth status

# Or use the recovery script:
bash /root/GITHUB_ACCESS_RECOVERY.sh
```

---

## ✅ Current Status

**Authentication:** ✅ Active and working
**Token Location:** `~/.config/gh/hosts.yml`
**Git Integration:** ✅ Configured via `gh auth setup-git`
**Last Verified:** 2025-12-21

**Recent PR Created:** https://github.com/x5th/x-leverage/pull/8

---

## 🔄 For Future Reference

If Claude Code or any future session needs to verify/restore access:

1. Read this file: `.claude/CRITICAL_GITHUB_ACCESS.md`
2. Get the token: `cat /root/.github_token_backup`
3. Authenticate: `cat /root/.github_token_backup | gh auth login --with-token`
4. Setup git: `gh auth setup-git`
5. Done!

**Or simply run:** `bash /root/GITHUB_ACCESS_RECOVERY.sh`

**This ensures access is NEVER lost.**
