# Week 3: rem CLI v0.7.0 — Deep Polish & Expansion

## Summary
53 source files, 22,825 LOC, 8 providers, 34+ slash commands. This plan targets **9 bugs**, **6 optimizations**, **5 new features**, **5 UI/UX improvements**, and **4 architecture items** over 7 days.

---

## Day 1 — Bug Fixes

### B1: build_retrieved_context appends footer twice
**File**: `rem-cli/src/indexer/mod.rs:196-200`
**Bug**: The footer `[End of retrieved context — use @path for more specific files if needed]` is pushed at line 196-197 and again at line 199-200 when `out.len() > 30`.
**Fix**: Remove the second redundant push. Keep only the first push after the `used > header_prefix.len()` check.
**Test**: Verify output doesn't contain duplicate footer.

### B2: REASONING_OVERIDE typo
**File**: `rem-cli/src/reasoning.rs:55`
**Bug**: `REASONING_OVERIDE` should be `REASONING_OVERRIDE`.
**Fix**: Rename to `REASONING_OVERRIDE`.
**Test**: Update all references.

### B3: paint() double no_color() check
**File**: `rem-cli/src/ui/theme.rs:346-349,427-439`
**Bug**: `paint()` calls `no_color()` first, then calls `t.fg()` which also calls `no_color()`. Two atomic reads per paint operation.
**Fix**: Pass a `colors_disabled: bool` parameter or restructure to avoid the double check. Simplest: make `fg()`/`bg()` not check no_color, let the caller handle it.
**Test**: Existing paint tests should still pass.

### B4: parse_history_turns fragile delimiter
**File**: `rem-cli/src/provider/mod.rs:564-594`
**Bug**: `parse_history_turns` uses `\nREM: ` as a boundary. If a user message contains "REM:", the parsing produces incorrect turns.
**Fix**: Use a more robust delimiter with a unique marker unlikely to appear in user content. Add a NUL separator or a UUID-based boundary.
**Test**: Add test case with user message containing "REM:" in content.

### B5: word_wrap breaks ANSI escape sequences
**File**: `rem-cli/src/repl.rs:880-905`
**Bug**: `word_wrap` splits lines at char boundaries using `floor_char_boundary` and space positions, but ANSI escape codes have zero visual width and get split mid-sequence.
**Fix**: Strip ANSI escape sequences before measuring width, then re-insert them after wrapping. Use a helper `visible_width()` function.
**Test**: Add test case with ANSI-colored text that wraps.

### B6: build_search_context allocates header for empty results
**File**: `rem-cli/src/chat.rs:344-353`
**Bug**: Always allocates `"Web search results:\n"` even when no results exist.
**Fix**: Return `String::new()` early if results are empty.
**Test**: Existing test already covers this.

### B7: looks_like_shell_command dead code
**File**: `rem-cli/src/blocklist.rs:152-158`
**Bug**: Function is defined but never imported/called anywhere.
**Fix**: Either remove it or add `#[allow(dead_code)]` if planned for future use. Remove to keep codebase clean.
**Test**: Compilation check.

### B8: build_prompt uses fragile ANSI markers
**File**: `rem-cli/src/session_io.rs:190-219`
**Bug**: Uses raw `\x01`/`\x02` markers for rustyline prompt — fragile.
**Fix**: Extract into a well-documented helper function.
**Test**: Visual inspection of prompt rendering.

### B9: duplicate footer text in build_retrieved_context
**File**: `rem-cli/src/indexer/mod.rs:167-202`
**Bug**: Footer text `[End of retrieved context...]` is pushed at both line 196 and line 199.
**Fix**: Remove duplicate at line 199-200.
**Test**: Verify single footer in output.

---

## Day 2 — Code Optimization & Quality

### O1: read_user_input duplicate storage
**File**: `rem-cli/src/repl.rs:156-157`
**Problem**: Both `combined: String` and `lines: Vec<String>` store the same text.
**Fix**: Remove `lines` Vec, use `combined.lines().count()` for line counting in prompt.
**Complexity**: Low

### O2: IndexChunk memory optimization
**File**: `rem-cli/src/indexer/mod.rs:77-85`
**Problem**: `content_lower`, `name_lower`, `path_lower` triple memory. For 50K chunks, this is substantial.
**Fix**: Compute lowered fields on-the-fly in BM25 tokenizer. Use `std::borrow::Cow` or `OnceLock<OnceCell>` for lazy init.
**Complexity**: Medium — touches BM25 retrieval path

### O3: stream_sse_response optimization
**File**: `rem-cli/src/provider/mod.rs:719-740`
**Problem**: Uses `trimmed.strip_prefix("data: ")` which allocates per line. Could use byte-level check.
**Fix**: Check bytes directly: `trimmed.as_bytes().starts_with(b"data: ")`.
**Complexity**: Low

### O4: _lower_buf misleading naming
**File**: `rem-cli/src/find.rs:262`
**Problem**: `let _lower_buf` with underscore prefix suggests unused, but it IS used via `haystack` reference.
**Fix**: Rename to `lower_buf`.
**Complexity**: Low

### O5: Reuse assistant_token_cache in build_chat_history
**File**: `rem-cli/src/chat.rs:138-162`
**Problem**: `build_chat_history()` re-estimates tokens every call instead of using cached values from `assistant_token_cache`.
**Fix**: Use `self.assistant_token_cache` when available instead of calling `estimate_tokens()` again.
**Complexity**: Low

### O6: display_performance_stats optimization
**File**: `rem-cli/src/repl.rs:992-1014`
**Problem**: Computes tokens/sec every response via division.
**Fix**: Minor — use `Duration::as_secs_f64()` directly (already).
**Complexity**: Trivial

---

## Day 3 — Testing Expansion

### T1: build_retrieved_context tests
- Footer is not duplicated
- Max chars boundary behavior
- Mixed chunk types rendering
- Empty input edge cases

### T2: parse_history_turns edge case tests
- User content containing "REM:" delimiter
- Empty histories
- Escaped newlines roundtrip
- Multi-turn with empty assistant messages
- Unicode content preservation

### T3: word_wrap ANSI-aware tests
- Text with ANSI escape codes wrapping correctly
- Zero-width characters (emoji + ANSI)
- Multi-byte + ANSI interaction
- Edge case: very long single word with ANSI

### T4: Blocklist unicode obfuscation tests
- Unicode confusables in dangerous commands
- Zero-width characters in patterns
- Mixed-script attack attempts

### T5: Provider retry logic edge cases
- `is_transient_error` with wrapped reqwest errors
- Stream cancellation during retry backoff
- Jitter randomization correctness

---

## Day 4 — New Features (Part 1)

### F1: GitHub Models provider
**File**: `rem-cli/src/provider/github.rs` (new)
**Description**: GitHub Models API — OpenAI-compatible, uses GitHub token. Base URL: `https://models.github.ai/v1` or `https://api.github.com/marketplace`.
**Implementation**:
1. Create `GitHubBackend` struct implementing `ProviderBackend`
2. Delegate to `openai_compat_*` functions
3. Add `GitHub` variant to `ProviderKind`
4. Add `GITHUB_TOKEN` / `GITHUB_API_KEY` env var
5. Update provider_from_str, provider_label, provider_supports_tools
**Complexity**: Low (~80 lines)

### F2: xAI Grok provider
**File**: Already OpenAI-compatible via OpenRouter, but add direct xAI support
**Description**: xAI API — base URL: `https://api.x.ai/v1`, model: `grok-2`.
**Implementation**: Very similar to GitHub Models, delegates to `openai_compat_*`.
**Complexity**: Low (~50 lines)

### F3: Streaming inline markdown rendering
**File**: `rem-cli/src/provider/mod.rs:642-651` (`emit_token`)
**Description**: Currently `emit_token()` writes raw text. Add lightweight inline Markdown rendering for the streaming path.
**Implementation**:
1. After each newline-flush, apply inline formatting: `` `code` `` → colored, `**bold**`, `*italic*`
2. Use a simple state machine per line
3. Only apply to non-code-fence lines
**Complexity**: Medium

---

## Day 5 — New Features (Part 2)

### F4: @ file reference tab completion
**File**: `rem-cli/src/completion.rs`
**Description**: When typing `@`, tab-complete file paths from the project directory.
**Implementation**:
1. Detect `@` prefix in completion context
2. Walk project directory for matching file paths
3. Return completions with appropriate icons
**Complexity**: Low

### F5: /context command
**File**: `rem-cli/src/commands/` (new handler)
**Description**: Show the exact prompt being sent to the LLM. Useful for debugging context injection.
**Output**: Full assembled prompt with character/token counts.
**Complexity**: Low (~40 lines)

### F6: Session analytics export
**File**: `rem-cli/src/chat.rs` + new handler
**Description**: Export token usage data: tokens per model, per provider, per session.
**Format**: JSON export of `{ provider, model, turn_count, total_tokens, duration_secs }`.
**Complexity**: Low

---

## Day 6 — UI/UX Improvements

### U1: ANSI-aware word_wrap
**File**: `rem-cli/src/repl.rs:880-905`
**Problem**: `word_wrap` doesn't account for ANSI escape codes having zero visual width.
**Fix**: Implement `visible_width()` that strips ANSI sequences before measuring.
**Complexity**: Medium

### U2: Multi-line editor support
**File**: `rem-cli/src/repl.rs:154-228`
**Problem**: No way to edit long multi-line prompts in external editor.
**Fix**: Add `/edit` command or `Ctrl+E` binding that opens `$EDITOR` on current buffer.
**Complexity**: Medium

### U3: Cache rendered markdown output
**File**: `rem-cli/src/repl.rs:937-989`
**Problem**: `display_text_output` re-renders markdown on every display.
**Fix**: Cache the last rendered output string and compare input.
**Complexity**: Low

### U4: Enhanced file display in code mode
**File**: `rem-cli/src/repl.rs:843-876`
**Problem**: File listing shows sizes but could show detected language and line count.
**Fix**: Add language tag and line count per file.
**Complexity**: Low

### U5: Consolidate no_color check
**File**: `rem-cli/src/ui/theme.rs:346-349`
**Problem**: `no_color()` checked in both `paint()` and `fg()`/`bg()`.
**Fix**: Remove check from `fg()`/`bg()`, keep only in `paint()` and other public functions.
**Complexity**: Low

---

## Day 7 — Architecture & Polish

### S1: MessagePack index format
**File**: `rem-cli/Cargo.toml` + `rem-cli/src/indexer/mod.rs`
**Description**: Add `rmp-serde` dependency. Support `.msgpack` format alongside JSON for 60-80% smaller index files and faster I/O.
**Implementation**:
1. Add optional `rmp-serde` dependency
2. On write, save both `.json` and `.msgpack`
3. On load, prefer `.msgpack` if available
4. Fall back to `.json` for backward compatibility
**Complexity**: Medium

### S2: Circuit breaker for provider retries
**File**: `rem-cli/src/provider/mod.rs:449-492`
**Description**: If a provider returns 5xx for 3 consecutive calls, stop retrying for a cooldown period.
**Implementation**:
1. Track consecutive failures per provider in a static map
2. On 3rd consecutive failure, skip retries for 30 seconds
3. Reset counter on success
**Complexity**: Low

### S3: Index loading with memmap
**File**: `rem-cli/src/indexer/mod.rs:94-163`
**Description**: Use `memmap2` crate for zero-copy index loading.
**Complexity**: Medium

### P1: CHANGELOG & README updates
**File**: `CHANGELOG.md`, `README.md`
**Description**: Document v0.7.0 changes.
**Complexity**: Low

### P2: Final verification
```bash
cargo check --all-targets
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```
**Complexity**: N/A

---

## Build & Test Verification

Run after completing all tasks:
```bash
cargo check                    # Fast type-check
cargo test                     # 560+ tests
cargo clippy --all-targets -- -D warnings  # Zero warnings
cargo build --release          # Release build
```

## Summary By Day

| Day | Focus | Items | Est. LOC |
|-----|-------|-------|----------|
| 1 | Bug Fixes | B1-B9 | ~150 |
| 2 | Code Optimization | O1-O6 | ~250 |
| 3 | Testing Expansion | T1-T5 | ~300 |
| 4 | New Features (Part 1) | F1-F3 | ~300 |
| 5 | New Features (Part 2) | F4-F6 | ~150 |
| 6 | UI/UX | U1-U5 | ~250 |
| 7 | Architecture & Polish | S1-S3, P1-P2 | ~300 |
| **Total** | | **~34 items** | **~1,700** |
