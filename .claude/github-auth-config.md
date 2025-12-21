# GitHub Authentication Configuration

## Status: ✅ Configured

GitHub CLI (gh) is authenticated and configured for this project.

## Configuration Location

Authentication is stored in: `~/.config/gh/hosts.yml`

## Git Configuration

Git is configured to use gh for authentication:
```bash
git config --global credential.helper
# Output: gh auth git-credential (via gh auth setup-git)
```

## How Authentication Works

1. **gh CLI**: Authenticated with Personal Access Token
2. **git operations**: Use gh CLI as credential helper
3. **Automatic**: No need to enter credentials for push/pull/PR operations

## Token Scopes

The configured token has full access including:
- repo (full control of private repositories)
- workflow
- admin:org, admin:repo_hook
- And other comprehensive scopes

## Verification

To verify authentication is working:

```bash
# Check gh authentication status
gh auth status

# Should show:
# ✓ Logged in to github.com account x5th
# - Token: ghp_************************************
```

## Usage

### Push to GitHub
```bash
git push origin <branch-name>
# No credentials needed - uses gh automatically
```

### Create Pull Request
```bash
gh pr create --base main --head <branch-name> --title "Title" --body "Description"
# or
gh pr create --fill  # Use commit message for title/body
```

### View Pull Requests
```bash
gh pr list
gh pr view <number>
```

## Troubleshooting

### "Authentication failed"
```bash
# Re-authenticate
gh auth login
# Choose: GitHub.com -> HTTPS -> Paste token
```

### "credential helper not working"
```bash
# Re-setup git integration
gh auth setup-git
```

## For Future Sessions

Authentication persists across terminal sessions and reboots because:
1. Token is stored in `~/.config/gh/hosts.yml` (permanent)
2. Git credential helper is configured globally (permanent)
3. No need to re-authenticate unless token is revoked

## Security Note

The token is stored securely in the gh configuration directory with restricted file permissions. Never commit tokens to git repositories.

---

**Last Updated:** 2025-12-21
**Status:** Active and working
**Token Expiration:** Check GitHub settings at https://github.com/settings/tokens
