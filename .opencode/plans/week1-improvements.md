# Week 1: rem CLI v0.4.0 — Foundation & Growth

## Day 1 — Critical Bug Fixes & Safety

### B1: Fix infinite loop in chunking.rs floor_char_boundary
**File**: `rem-cli/src/indexer/chunking.rs:31`
**Problem**: When `floor_char_boundary(end)` returns 0 (multi-byte char at position 0), `start = end = 0` causes infinite loop.
**Fix**: After `floor_char_boundary`, if `end == start`, advance by at least 1 char boundary.
**Test**: Add test case with multi-byte prefix string, e.g., `"éabc"` with small target.

### B2: Fix reasoning.rs system_prompt_not_supported blanket exclusion
**File**: `rem-cli/src/reasoning.rs:72-75`
**Problem**: `starts_with("o1-")` catches ALL o1-* models including `o1-2024-12-17` which does support system prompts.
**Fix**: Narrow to only `o1-preview` and `o1-mini` (the actual models without system prompt support).
**Test**: Update test cases.

### B3: Fix STREAM_CANCELLED not cleared between streams
**File**: `rem-cli/src/provider/mod.rs`
**Problem**: `STREAM_CANCELLED` is set by Ctrl+C but only cleared in the REPL loop path. Non-REPL code paths (`commands/tools.rs`, `commands/goal.rs`, `runner.rs`) never reset it, causing subsequent provider calls to immediately fail.
**Fix**: Reset `STREAM_CANCELLED` at the start of each `Provider::complete_chat_stream`, `complete_json`, `complete_chat_stream_with_vision`, `complete_chat_stream_with_tools` method.

### B4: Audit find.rs haystack scoping (confirmed safe)
**File**: `rem-cli/src/find.rs:249-258`
**Assessment**: `haystack` and `_lower_buf` are declared in the same `for` loop iteration scope. The borrow checker prevents use-after-free. No code change needed — add clarifying comment.

### B5: Replace unwrap() calls in config.rs
**File**: `rem-cli/src/config.rs`
**Problem**: Several `unwrap()` calls on fallible operations (config load, env var parse, dir creation).
**Fix**: Convert to proper error propagation with `?` or `context()`.

### B6: Fix ollama.rs vision history placeholder validation
**File**: `rem-cli/src/provider/ollama.rs:187-189`
**Problem**: Pops last message but doesn't validate it's the expected empty user placeholder.
**Fix**: Check that the last message is the expected placeholder before popping.

---

## Day 2 — Code Quality & Optimization

### Q1: Optimize days_to_year() iterative loop
**File**: `rem-cli/src/text_util.rs:45-66`
**Problem**: O(Y) loop for year calculation.
**Fix**: Replace with arithmetic formula using 400-year blocks.
**Test**: Verify same output for known dates.

### Q2: Optimize normalize_cmd() iterative convergence
**File**: `rem-cli/src/blocklist.rs:8-23`
**Problem**: Loops until stable via repeated split+join.
**Fix**: Single-pass regex `\s+` replacement.

### Q3: Lazy init for intent phrase combinations
**File**: `rem-cli/src/intent.rs:160-164`
**Problem**: `VERB_PHRASES_SPACE` and `PHRASE_COMBINATIONS_SPACE` eagerly allocate N*M strings at module init.
**Fix**: Use `std::sync::LazyLock` for lazy initialization.

### Q4: Overflow guard in human_size()
**File**: `rem-cli/src/text_util.rs:5-13`
**Problem**: No overflow protection for very large u64 values.
**Fix**: Cap at `u64::MAX` display.

### Q5: Cache file_icon lowercase conversions
**File**: `rem-cli/src/types.rs` (file_icon_for)
**Problem**: Calls `.to_lowercase()` on every lookup, allocating per file.
**Fix**: Use a `OnceLock<HashMap<String, &str>>` for pre-computed lowercase keys.

### Q6: Wire up AnthropicDelta.thinking field
**File**: `rem-cli/src/provider/anthropic.rs:73-74`
**Problem**: `#[allow(dead_code)]` on `thinking` field.
**Fix**: Wire into the `/reasoning show` path or remove the annotation if truly dead.

### Q7: Extract classify_intent_heuristic()
**File**: `rem-cli/src/intent.rs:186-253`
**Problem**: ~70-line function with many boolean conditions.
**Fix**: Extract into smaller focused functions (one per intent type check).

---

## Day 3 — Testing Expansion

### T1: find.rs tests
- Case-sensitive/insensitive search behavior
- max_results truncation
- Empty result sets
- Regex cache behavior
- Temp dir cleanup on failure (fix existing bugs)

### T2: config.rs tests
- Config load from file / missing file / corrupted file
- TOML parsing edge cases
- XDG fallback paths
- Env var resolution

### T3: provider/mod.rs tests
- `parse_history_turns` edge cases (empty, single turn, multi turn, escaped newlines)
- `build_messages_from_history`
- `is_transient_error` (timeout, 429, 500, connection refused)
- `add_openai_auth` per provider kind
- `stream_buf` with wiremock

### T4: indexer/bm25.rs tests
- BM25 scoring correctness
- Tokenization edge cases (unicode, empty docs, stop words)

### T5: indexer/chunking.rs tests
- Empty files, single-char files, null bytes, multi-byte splits

### T6: indexer/mod.rs tests
- Full index cycle: walk → chunk → save → load → search
- Temp file tree with various extensions

---

## Day 4 — New Features

### F1: Incremental indexing
**File**: `rem-cli/src/indexer/mod.rs`
**Description**: Track file mtimes. On re-index, only reprocess files whose mtime changed.
**Status**: ~2hr, medium complexity.

### F2: Custom error types for provider
**File**: `rem-cli/src/provider/mod.rs`
**Description**: Create `ProviderError` enum (Auth, RateLimit, Timeout, ServerError, ParseError) to replace bare `anyhow::Error`.
**Status**: ~2hr, touches all provider files.

### F3: Configurable page threshold
**File**: `rem-cli/src/pager.rs` + `rem-cli/src/cli.rs`
**Description**: Add `PAGE_THRESHOLD` config field to make the 50-line pager threshold configurable.

### F4: Session export/import
**File**: `rem-cli/src/commands/session.rs`
**Description**: `/session export <path>` and `/session import <path>` for sharing/backing up sessions.

---

## Day 5 — UI/UX Improvements

### U1: Progress bar for indexing
**File**: `rem-cli/src/indexer/mod.rs`
**Description**: Show file count, elapsed time, estimated remaining using indicatif crate (optional dependency).

### U2: Better markdown rendering
**File**: `rem-cli/src/ui/markdown.rs`
**Description**: Support tables, task lists (-[x]), strikethrough.

### U3: Extended syntax highlighting
**File**: `rem-cli/src/highlight.rs`
**Description**: Add Python, Rust, Go, JSON highlighting (currently only HTML/CSS/JS).

### U4: Startup banner with model info
**File**: `rem-cli/src/repl.rs:588`
**Description**: Show `Provider/Model` in welcome banner.

### U5: Better unknown command suggestions
**File**: `rem-cli/src/commands/mod.rs`
**Description**: Surface `did_you_mean` more prominently.

### U6: REPL header enhancements
**File**: `rem-cli/src/ui/header.rs`
**Description**: Add token count, cost estimate, session duration.

---

## Day 6 — Scaling & Architecture

### S1: BM25 inverted index
**File**: `rem-cli/src/indexer/bm25.rs`
**Description**: Replace linear scan with term→doc_ids map for sub-linear queries.

### S2: Chunked index I/O
**File**: `rem-cli/src/indexer/mod.rs`
**Description**: Split `codebase_index.json.gz` into per-directory shards.

### S3: Connection pool tuning
**File**: `rem-cli/src/provider/mod.rs`
**Description**: Make `pool_max_idle_per_host` configurable, default 8.

### S4: History sliding window
**File**: `rem-cli/src/chat.rs`
**Description**: Add `max_history_tokens` config to trim old turns when memory exceeds limit.

---

## Day 7 — Polish, CI & Documentation

### P1: Add cargo audit + cargo deny to CI
**File**: `.github/workflows/ci.yml`
**Description**: Security vulnerability scanning and license compliance.

### P2: Add Justfile targets
**File**: `Justfile`
**Description**: Add `audit`, `outdated`, `bench` targets.

### P3: Normalize error output
**Description**: Audit all `tracing::warn!` vs `eprintln!` vs `println!` usage across codebase. Normalize to `tracing` for logs and `ui::theme::` for user output.

### P4: Update README
**Description**: Document new features (incremental index, session export, progress bar).

### P5: Prepare CHANGELOG
**Description**: Draft v0.5.0 release notes.

### P6: API key redaction in debug logs
**File**: `rem-cli/src/provider/mod.rs:584`
**Description**: Redact API keys from debug/error log output.

---

## Build & Test Verification

```bash
# Run after completing all tasks
cargo check                    # Fast type-check
cargo test                     # All 207+ tests
cargo clippy --all-targets -- -D warnings  # Zero warnings
cargo build --release          # Release build
```
