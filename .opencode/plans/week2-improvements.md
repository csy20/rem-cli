# Week 2: rem CLI v0.6.0 — Deep Debug & Expansion

## Summary

53 source files, 22,825 LOC, 7 providers, 40 slash commands. This plan targets **7 bugs**, **6 optimizations**, **4 new features**, **5 UI/UX improvements**, and **4 polish items** over 7 days.

---

## Day 1 — Critical Bug Fixes

### B1: web_search tool ignores configured search provider
**File**: `src/tool_executor.rs:189`
**Bug**: `execute_web_search()` passes `None` to `perform_web_search()`, bypassing the user's configured `search_provider` in `AppConfig`. The function doesn't have access to the config.
**Fix**:
1. Add `search_provider: Option<String>` parameter to `execute_web_search()`
2. Thread the configured provider through `execute_tool_call()` → needs config access
3. Add `config: &AppConfig` parameter to `execute_tool_call()` or use `CONFIG_CACHE`
4. Pass `config.search_provider` when calling `perform_web_search()`
**Complexity**: Medium — touches signature chain across `tool_executor.rs` and `run_tool_loop()`

### B2: run_command approval hangs in non-interactive/pipe mode
**File**: `src/tool_executor.rs:267-271`
**Bug**: `execute_run_command()` always prints `[y/N]` prompt and calls `approve_fn`, which in pipe mode reads from stdin. No user present → hangs indefinitely.
**Fix**:
1. Check `atty::is(atty::Stream::Stdin)` or use `std::io::stdin().is_terminal()` (nightly) / a cached `is_interactive` flag
2. Pass `is_interactive: bool` through the execution chain
3. If non-interactive, auto-deny with message "shell command not allowed in non-interactive mode"
**Alternative**: Add `auto_approve_shell` config option
**Complexity**: Medium — requires plumbing `is_interactive` through `execute_tool_call()` and `run_tool_loop()`

### B3: Ask user tool blocks the REPL
**File**: `src/tool_executor.rs:438-463`
**Bug**: `execute_ask_user()` reads from `std::io::stdin().read_line()` directly. In REPL context, this bypasses rustyline, breaking the input state (history, multi-line, bracket handling).
**Fix**:
- Add `ask_fn: &mut dyn FnMut(&str) -> Option<String>` parameter to `execute_tool_call()`
- In REPL mode, pass a closure that uses rustyline to read input
- In CLI mode, pass the current `std::io::stdin().read_line()` behavior
**Complexity**: Medium

### B4: No input size validation for vision images
**File**: `src/vision.rs`
**Bug**: `encode_image()` reads the entire file into memory. A large image (e.g., 500MB) causes OOM or provider rejection.
**Fix**:
1. Add `MAX_VISION_IMAGE_BYTES = 20_000_000` (20 MB) to `constants.rs`
2. Check file size via `std::fs::metadata()` before reading
3. Return user-friendly error if exceeded
**Complexity**: Low

### B5: edit_file replaces LAST occurrence (ambiguous semantics)
**File**: `src/tool_executor.rs:342-347`
**Bug**: `execute_edit_file()` uses `rfind` (last match) when `count > 1`. Most users/LLMs expect first-match replacement. The behavior is counter-intuitive.
**Fix**:
- Change `rfind` to `find` (always replace first occurrence)
- Or add `occurrence: usize` parameter to tool spec, defaulting to 0 (first)
**Complexity**: Low

### B6: Race condition in Memory file writes
**File**: `src/memory.rs`
**Bug**: `save()` and `append()` write to `.rem/memory.md` without file locking. Concurrent rem processes can corrupt the file.
**Fix**:
1. Use `fs2::FileExt::lock_exclusive()` (or `fd-lock` crate) for advisory locking
2. Lock before read, write after lock
3. Unlock on drop
**Complexity**: Low — add `fd-lock` dependency, ~20 lines of changes

### B7: Gemini tool calls use synthetic IDs (potential collision)
**File**: `src/provider/gemini.rs` (tool call handling)
**Bug**: Tool call IDs are generated as `format!("fc_{}", name)` rather than being server-assigned. If the response contains two `read_file` calls, they'd share the same ID.
**Fix**:
- Use an `AtomicU32` counter (or UUID) instead of name-based IDs
**Complexity**: Low

---

## Day 2 — Code Optimization & Quality

### O1: Lazy index loading for faster startup
**File**: `src/indexer/mod.rs:94-163` (load function), `src/repl.rs` (startup path)
**Problem**: `load_codebase_index()` is called during session initialization, adding ~50-200ms to startup even when indexing isn't needed.
**Fix**: Defer index loading until first BM25 query (`retrieve_relevant_chunks()`). Expose `lazy_load()` method that returns `Option<Arc<CodebaseIndex>>`.
**Complexity**: Medium

### O2: Cache token estimates in build_chat_history
**File**: `src/chat.rs:143-162`
**Problem**: `estimate_tokens()` is called per turn every time history is assembled. On long sessions, this re-estimates the same strings repeatedly.
**Fix**:
1. `assistant_token_cache: Vec<usize>` already exists (line 45) but is only used by `push_turn()` for eviction
2. Extend caching: when building history string, reuse cached estimates instead of re-calling `estimate_tokens()`
3. Invalidate cache on push (already done) and compact
**Complexity**: Low

### O3: Parallel independent tool execution
**File**: `src/tool_executor.rs` (run_tool_loop)
**Problem**: When the LLM returns multiple tool calls, they're executed sequentially. Independent calls (e.g., two `read_file` calls) could run in parallel.
**Fix**:
1. Group tool calls by independence (no write-after-read conflicts or same-file writes)
2. Use `tokio::join!` or `futures::future::join_all` for parallel execution
3. Collate results in original order
**Complexity**: Medium

### O4: Anthropic stream parsing deduplication
**File**: `src/provider/anthropic.rs:353-410` vs `src/provider/mod.rs:734-802`
**Problem**: `complete_chat_stream_with_tools()` has its own inline stream parser that partially duplicates `stream_anthropic_sse()` logic.
**Fix**: Refactor to share common delta parsing via a callback/visitor pattern
**Complexity**: Medium

### O5: Retry jitter randomization
**File**: `src/provider/mod.rs:462-466`
**Problem**: Jitter uses `DefaultHasher` on attempt number, producing deterministic values. This creates thundering herd when multiple clients retry simultaneously.
**Fix**: Use `rand::thread_rng().gen_range()` for true random jitter. Add `rand` dependency (small, already in Cargo.lock transitively? Check).
**Complexity**: Low

### O6: Deduplicate `first_run_setup` config logic
**File**: `src/config.rs` (look for `first_run_setup` or similar startup logic)
**Problem**: Config loading and validation have some duplicated checks between `load_and_validate()` and inline code paths.
**Fix**: Consolidate config loading into a single pipeline
**Complexity**: Low

---

## Day 3 — Testing Expansion

### T1: tool_executor.rs tests
**File**: `src/tool_executor.rs`
**Current**: 0 unit tests (only integration through the tool system)
**Add**:
- `web_search` with mocked search results (mock the HTTP client)
- `edit_file` first-vs-last occurrence behavior
- `run_command` with approval denied / approved / timeout
- `ask_user` response handling
- Path traversal blocking
- `extract_arg` edge cases (missing, wrong type, null)
**Complexity**: Medium (needs HTTP mocking or trait extraction)

### T2: Blocklist edge case tests
**File**: `src/blocklist.rs` (add to existing test module)
**Add**:
- Unicode obfuscation in blocked commands (e.g., `r\x6d -rf /` — hex escape)
- Mixed-case blocked patterns
- Whitespace-only and empty commands
- `normalize_cmd` with control characters, null bytes, backslash escapes
- Multi-byte Unicode normalization edge cases
**Complexity**: Low

### T3: Memory concurrent access tests
**File**: `src/memory.rs`
**Add**:
- Simulate concurrent `append()` from two threads
- Verify file content isn't corrupted
- Lock contention behavior
**Complexity**: Low

### T4: Vision edge case tests
**File**: `src/vision.rs`
**Add**:
- File size exceeded (mock metadata)
- Invalid image format detection
- Very large base64 output
- MIME type detection for edge extensions (.jpeg, .JPG, .PNG)
**Complexity**: Low

### T5: HistoryManager eviction tests
**File**: `src/chat.rs`
**Current**: Token-based eviction exists but isn't well-tested
**Add**:
- Eviction when `max_history_tokens` is exceeded
- No eviction when under budget
- Empty history handling
- Single turn overflow
- `set_max_history_tokens_from_ctx` edge cases
**Complexity**: Low

---

## Day 4 — New Features

### F1: DeepSeek provider
**File**: `src/provider/deepseek.rs` (new), plus modifications to `provider/mod.rs`, `config.rs`, `reasoning.rs`
**Description**: DeepSeek offers two models that need distinct handling:
- `deepseek-chat` (V3) — standard conversational, supports streaming + tools
- `deepseek-reasoner` (R1) — reasoning model, no system prompt, needs chain-of-thought extraction

**Implementation**:
1. Create `src/provider/deepseek.rs`:
   - `DeepSeekBackend` struct (similar to `OpenAIBackend` — API is OpenAI-compatible)
   - Override `complete_chat_stream()`: for `deepseek-reasoner`, handle the `reasoning_content` field in the delta
   - Override `complete_chat_stream_with_tools()`: for `deepseek-chat`, delegate to `openai_compat_*`; for `deepseek-reasoner`, return `ToolResponse::Text` (no tool support)
   - Override `complete_json()` using `openai_compat_complete_json`
2. Add `DeepSeek` variant to `ProviderKind` enum (provider/mod.rs:158-166)
3. Add default base URL `https://api.deepseek.com` and model `deepseek-chat` to defaults table
4. Add `DEEPSEEK_API_KEY` env var lookup
5. Update `provider_from_str()`, `provider_label()`, `provider_supports_tools()`
6. Update `reasoning.rs:is_reasoning_model()` to include `"deepseek-reasoner"` and extract `reasoning_content` in streaming
7. Add to config validation's known provider list

**Complexity**: High (~250 lines new code, ~50 lines modifications across existing files)

### F2: Provider health check
**File**: `src/commands/repl.rs` (new `/ping` handler), `src/provider/mod.rs` (new `ping()` method on Provider)
**Description**: Quick connectivity test for the active provider.
**Implementation**:
1. Add `async fn ping(&self) -> Result<()>` to `ProviderBackend` trait with default impl that calls `list_models()` and checks response time
2. Add `/ping` command to the registry (Session category)
3. Display: latency, model count, API version if available
**Complexity**: Low

### F3: Prompt templates
**File**: `src/commands/session.rs` or new file, plus `.rem/prompts/` directory
**Description**: Save and reuse prompt templates. Useful for recurring tasks.
**Commands**:
- `/prompt save <name>` — save current input as template
- `/prompt load <name>` — load and insert template
- `/prompt list` — list saved templates
- `/prompt delete <name>` — remove template

**Implementation**:
1. Directory `.rem/prompts/` stores `.md` files
2. Templates support `{{variable}}` placeholders for substitution
3. `prompt_save()` writes, `prompt_load()` reads and optionally prompts for variables
**Complexity**: Medium

### F4: Custom system prompt override
**File**: `src/config.rs` (load path), `src/session_io.rs` (prompt building)
**Description**: Allow `.rem/system_prompt.md` to override the built-in system prompt per project.
**Implementation**:
1. After loading the standard system prompt, check for `.rem/system_prompt.md`
2. If found, use it as the base system prompt (or append to it, configurable)
3. Document in `/init` output
**Complexity**: Low

---

## Day 5 — UI/UX Improvements

### U1: Streaming Markdown rendering
**File**: `src/provider/mod.rs` (`emit_token`) and `src/repl.rs` (display pipeline)
**Problem**: Currently `emit_token()` writes raw text to stdout. Bold/italic/code appear as raw `*`/`_`/backtick markers.
**Fix**: Add lightweight inline Markdown rendering to the streaming path:
1. After each newline-flush in `emit_token()`, apply inline formatting to the accumulated line:
   - `` `code` `` → colored text
   - `**bold**` → bold text
   - `*italic*` → dim text
2. Use a simple state machine (not full parser) for performance
3. Only apply to non-code-fence lines

**Alternative**: Buffer lines and apply rendering on newline boundaries. More correct, slightly more latency.
**Complexity**: Medium

### U2: Multi-line prompt editor
**File**: `src/repl.rs:154-228` (`read_user_input`)
**Description**: For composing long multi-line prompts (system design docs, complex refactoring), open `$EDITOR` or a built-in editor widget.
**Implementation**:
1. Detect `/edit` or `Ctrl+E` keybinding in rustyline
2. Write current buffer to temp file
3. Spawn `$EDITOR` process
4. Read back and continue
5. If no `$EDITOR`, show instructions for multi-line mode
**Complexity**: Medium

### U3: /status dash command
**File**: `src/commands/repl.rs` (new handler), `src/provider/mod.rs` (usage stats)
**Description**: Single overview command showing everything about the current session.
**Output**:
```
Provider:   anthropic/claude-sonnet-4-20250514
Mode:       CHAT
Model ctx:  200K
Tokens:     1,234 this turn | 9,876 total | 45% of window
Session:    32m 14s | 23 turns
Ping:       340ms
Index:      1,024 chunks | 12 files indexed
```

**Implementation**:
1. Add `/status` to registry
2. Gather from `ChatSession` (tokens, duration, turns), `Provider` (latency from cache), `CodebaseIndex` (chunk/file count)
3. Use theme-aware formatting
**Complexity**: Low

### U4: Progress bar for indexing
**File**: `src/indexer/mod.rs` (generate_codebase_index)
**Description**: Show file count, elapsed time, and estimated remaining during `rem index` or auto-indexing.
**Implementation**:
1. Add `indicatif` as optional dependency (already in spirit, check if it's a lightweight dep)
2. Before the parallel chunking pass, create a `ProgressBar` with count of new/changed files
3. Tick per file processed
4. On completion, show summary (N files, X chunks, Y seconds)
5. Respect `NO_COLOR` env var
**Complexity**: Medium

### U5: Tab-complete file paths and model names
**File**: `src/completion.rs`
**Current**: Tab completion exists but is basic (mostly command names)
**Extend**:
1. After `/find`, `/write`, `/dir`, `/explain`, `/test`, `/refactor`, tab-complete file paths from the project
2. After `/model`, tab-complete model names from the provider's model list
3. After `/theme`, tab-complete theme names
4. After `/provider`, tab-complete provider names
**Complexity**: Medium

---

## Day 6 — Scaling & Architecture

### S1: Plugin system v1 for tools
**File**: New `src/plugin/` module
**Description**: Load external tool implementations from TOML manifests, enabling user-defined tools without modifying rem-cli source.
**Implementation**:
1. Define `PluginManifest` TOML format:
   ```toml
   [tool]
   name = "deploy_to_vercel"
   description = "Deploy the project to Vercel"
   command = "vercel"
   args = ["--prod"]
   timeout_s = 120
   ```
2. Load from `.rem/plugins/*.toml`
3. Register external tools alongside built-in tools
4. Execution via `run_command`-like subprocess (with approval)
5. Results passed back as tool output
**Complexity**: High

### S2: Background indexing daemon
**File**: `src/indexer/mod.rs`, `src/watcher.rs`
**Description**: On REPL startup, spawn a background task that walks files and builds/updates the index without blocking input.
**Implementation**:
1. On `initialize_session()`, spawn `tokio::task::spawn_blocking()` for indexing
2. Use `watch::Receiver` to signal when done
3. Store result in `Arc<RwLock<Option<CodebaseIndex>>>`
4. Mark index as "stale" when watcher detects file changes
5. Auto-rebuild when idle (3s after last file change)
**Complexity**: High

### S3: Unified config editor
**File**: `src/commands/session.rs` (new `/config edit` handler)
**Description**: Open the config file in `$EDITOR` for interactive editing.
**Implementation**:
1. Launch `$EDITOR` on `~/.config/rem-cli/config.toml`
2. After editor exits, validate and reload config
3. Report any validation warnings
**Complexity**: Low

### S4: Atomic config saves with rollback
**File**: `src/config.rs`
**Problem**: Config writes (via `persist_workspace` and `config_set`) could leave corrupted files on crash.
**Fix**: Write to `.tmp` file, then `fs::rename()` for atomic replacement. Restore from backup on validation failure.
**Complexity**: Low

---

## Day 7 — Polish & CI

### P1: Release CHANGELOG v0.6.0
**File**: `CHANGELOG.md`
**Description**: Document all changes from this week.
**Complexity**: Low

### P2: README updates
**File**: `README.md`
**Description**: Add DeepSeek provider, new commands (`/ping`, `/status`, `/prompt`), plugin system docs.
**Complexity**: Low

### P3: Justfile updates
**File**: `Justfile`
**Add**: `fuzz`, `bench-compare`, `check-all` (runs check + test + clippy + audit)
**Complexity**: Low

### P4: CI hardening
**File**: `.github/workflows/ci.yml`
**Add**:
- `cargo audit` (already present, make blocking not continue-on-error)
- `cargo deny` check for license + duplicate deps
- Benchmark comparison gate (warn if >5% regression)
- Fuzz testing step (optional, 1m timeout)
**Complexity**: Medium

### P5: Final verification
```bash
cargo check --all-targets
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
cargo audit
cargo deny check
```
**Complexity**: N/A

---

## Build & Test Verification

Run after completing all tasks:
```bash
cargo check                    # Fast type-check
cargo test                     # 540+ tests
cargo clippy --all-targets -- -D warnings  # Zero warnings
cargo build --release          # Release build
cargo audit                    # No known vulnerabilities
```

---

## Summary By Day

| Day | Focus | Items | Est. LOC |
|-----|-------|-------|----------|
| 1 | Critical Bug Fixes | B1-B7 | ~200 |
| 2 | Code Optimization | O1-O6 | ~250 |
| 3 | Testing Expansion | T1-T5 | ~300 |
| 4 | New Features | F1-F4 | ~500 |
| 5 | UI/UX | U1-U5 | ~400 |
| 6 | Scaling & Architecture | S1-S4 | ~600 |
| 7 | Polish & CI | P1-P5 | ~100 |
| **Total** | | **~35 items** | **~2,350** |
