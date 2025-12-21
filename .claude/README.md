# .claude Directory

This directory contains Claude Code-specific configuration and instructions.

## Files

- **build-config.md** - Critical build instructions for Claude Code
  - Specifies correct Solana toolchain to use
  - Provides correct build commands
  - Prevents dependency resolution errors

## For Claude Code Sessions

When starting work on this project:
1. Read `.claude/build-config.md` for build requirements
2. Always use the specified PATH when building
3. Prefer using `./build.sh` for builds

## For Humans

These files document the correct build process and serve as reference for Claude Code to ensure consistent, correct builds across sessions.
