# AGENTS.md — rem-cli

## Environment

- **Language:** Rust (edition 2021)
- **Build:** `cargo build`
- **Test:** `cargo test` (153 unit + 17 integration + 12 intent_parsing tests)
- **Lint:** `cargo clippy` (zero warnings target)
- **Run:** `cargo run -- [args]` or `./target/debug/rem`

## Project Structure

```
src/
├── main.rs          — Entry point, CLI dispatch, REPL loop, Ctrl+C handler
├── cli.rs           — Clap argument parsing, AppConfig & PartialConfig
├── config.rs        — Config loading/saving, provider construction
├── chat.rs          — ChatSession (history, state, serialization)
├── repl.rs          — Interactive REPL loop
├── provider/        — LLM providers (Ollama, OpenAI, Anthropic, Gemini)
│   ├── mod.rs       — Provider enum, shared Client, stream handlers
│   └── gemini.rs    — Gemini-specific streaming
├── indexer.rs       — Codebase indexing (rem index) + keyword retrieval
├── intent.rs        — Query intent classification
├── commands/        — REPL slash command handlers
│   ├── mod.rs
│   ├── files.rs     — /write, /undo, /copy
│   ├── session.rs   — /dir, /config, /memory, /save
│   ├── tools.rs     — /search, /explain, /test, /refactor, /lint, /find
│   ├── goal.rs      — /goal autonomous loop
│   ├── review.rs    — /diff, /review
│   └── help.rs      — /help
├── templates.rs     — Project scaffolding templates
├── token_count.rs   — Token estimation
├── types.rs         — Shared types (FileEntry, ModelReply, resolve_safe_path)
├── find.rs          — Filesystem text search
├── search.rs        — Web search (DuckDuckGo)
├── parsing.rs       — Code fence extraction
├── agentic.rs       — Agentic loop (goal orchestration)
├── memory.rs        — Project memory persistence
├── pager.rs         — Pager output
├── highlight.rs     — Syntax highlighting
├── feedback.rs      — User feedback
└── ui/              — Terminal UI
    ├── mod.rs
    ├── theme.rs     — Color themes (GHOST, etc.)
    ├── markdown.rs  — Markdown rendering
    └── output.rs    — print_banner, print_reply, SpinnerGuard
```

## Build & Test Commands

```bash
cargo test                    # Run all tests
cargo test -- --nocapture     # Run with stdout visible
cargo clippy                  # Lint check (zero warnings)
cargo build                   # Debug build
cargo build --release         # Release build
cargo check                   # Fast type-check only
```

## Code Conventions

- `pub(crate)` visibility for cross-module but not public API
- `anyhow::Result` for fallible functions
- Theme-aware terminal output via `ui::theme::` helpers (never raw ANSI)
- Ctrl+C: uses `CTRL_C_COUNT` + `SHOULD_EXIT` atomics in `main.rs`
- Stream cancellation: `provider::STREAM_CANCELLED` atomic
- Lock poisoning: always use `unwrap_or_else(|e| e.into_inner())` on mutexes
- Error labels: `ui::theme::paint_error_label()`, success: `paint_success_label()`
- Import style: `use crate::` for internal, grouped by module
- Tests: `#[cfg(test)] mod tests { use super::*; }` at end of source file
- New features must keep all tests passing and clippy clean
