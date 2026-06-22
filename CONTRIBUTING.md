# Contributing to rem

Thanks for your interest in contributing! rem is a solo project, but issues,
PRs, and ideas are welcome.

## Code of conduct

Be respectful and constructive. This is a beginner-friendly project.

## How to contribute

### Reporting bugs

Open an issue using the bug report template. Include:
- Steps to reproduce
- Expected vs actual behavior
- Your OS, rem version, and model name

### Suggesting features

Open an issue using the feature request template. Describe the problem you're
solving, not just the solution you want.

### Pull requests

1. Fork the repo and create a branch from `main`.
2. Run `cargo fmt` and `cargo clippy` before committing (zero warnings).
3. Add tests for new functionality.
4. Update `CHANGELOG.md` if applicable.
5. Open a PR against `main`.

## Development setup

```bash
cd rem-cli
cargo build
cargo test
cargo clippy --all-targets  # must be clean
```

## Code conventions

See `rem-cli/AGENTS.md` for full conventions. Key rules:
- `pub(crate)` visibility for cross-module API
- `anyhow::Result` for fallible functions
- Theme-aware terminal output (no raw ANSI)
- Tests at end of source file with `#[cfg(test)] mod tests`
- Streaming cancellation via `STREAM_CANCELLED` atomic
- Lock poisoning: `unwrap_or_else(|e| e.into_inner())`

## Questions

Open a discussion or issue. I'll respond when I can.
