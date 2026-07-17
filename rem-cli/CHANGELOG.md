# Changelog

## v0.8.0 (unreleased)

### New Features
- **`/compare` command**: Send the same prompt to multiple models/providers and compare responses side-by-side. (#F6)
- **`/session analytics`**: Export session analytics as JSON (provider, model, turn count, tokens, duration). (#F2)
- **`/session list` enhancement**: Now shows timestamps, turn counts, token counts, and session duration. (#F3)
- **`/git status`, `/git diff`, `/git log`**: Quick git workflow commands without leaving the REPL. (#F7)
- **`/prompt` template system**: Save, load, list, and delete reusable prompt templates with `{{variable}}` substitution. (#F1)
- **Plugin system v1**: Lightweight plugin trait with `PluginManager`, built-in `hello` plugin, and `/plugin` command. (#F8)
- **Session model tracking**: `ChatSession` now stores the model name, persisted in session JSON and exported in analytics. (#F2)

### Bug Fixes
- **`visible_width()` rewrite**: Properly handles CSI, OSC, DCS, and two-byte ANSI sequences (was only handling sequences ending in `'m'`). (#B1)
- **`looks_like_shell_command` restored**: Was incorrectly flagged as dead code and removed — now re-added with original implementation. (#B2)
- **`execute_list_files` error handling**: Replaced `unwrap_or(false)` on `file_type()` with proper `match`. (#B3)
- **`search.rs` regex safety**: `.expect()` calls replaced with `.ok()` + `Option<Selector>` pattern. (#B4)
- **`NoopUserInteraction` borrow fix**: Changed from `let mut noop = ...` to inline reference. (#B5)
- **Config field clone fix**: Used `.clone().unwrap_or_default()` to avoid moving from borrowed context in tool_executor. (#B6)

### UI/UX
- **Error display**: LLM errors now show the provider label alongside the error message.
- **`/status` enhancement**: Now displays the provider's base URL.
- **`/session analytics` enrichment**: Includes actual provider name instead of "unknown".
- **`/session analytics` parameters**: Accepts optional file path to write JSON output.

### Testing
- **47 new unit tests**: Coverage expanded for completion, constants, providers (GitHub, xAI), visible_width/skip_ansi/word_wrap, session auto-restore, prompt commands, compare, git, plugin, and fuzz stress tests.
- **3 fuzz-like property tests**: Random ANSI sequences for `visible_width`, random inputs for `word_wrap`, random brackets for `needs_continuation`.
- **Total test count**: 660 unit tests + 18 integration tests = 678 total.

### Architecture
- **Plugin system**: Extensible `Plugin` trait with `PluginManager` registry, integrated into REPL via `/plugin` command. (#F8)
- **CI benchmark gate**: Added `cargo bench --no-run` step to CI workflow.
- **Fuzz-like tests**: Property-based stress tests for critical text processing functions.

## v0.7.0 (unreleased)

### Bug Fixes

- **Safe Mmap**: Added SAFETY comments to both `Mmap::map` calls in `indexer/mod.rs`. (#P6)
- **Gemini API key redaction**: All 4 `parse_api_error` calls now pass `Some(ctx.api_key_str())` instead of `None`. (#P6)
- **Config TOCTOU race**: Added generation counter to detect stale cache entries in `load_config`/`save_config`. (#B8)
- **Language detection ordering**: Fixed `detect_language_from_content` in `highlight.rs` — Ruby `class` check no longer matches Python classes (lacked colon check); Go `package` check excludes Java-style `package com.example;`. (#B9)

### New Features

- **Session auto-restore**: REPL now auto-resumes the last saved session from `.rem/session.json.gz` on startup (opt-out via `auto_resume=false` in config). (#F5)
- **Provider did-you-mean**: `/provider` and config validation suggest close provider names on typo (Levenshtein distance ≤ 2). (#U7)
- **Web search cache**: In-memory cache with 5-minute TTL for search results, reducing duplicate network calls. (#O6)

### UI/UX

- **Index progress indicator**: Scanning phase shows elapsed time; chunking phase shows file count and duration. (#U8)
- **Better HTTP error messages**: `parse_api_error` now includes canonical status text (e.g., "401 Unauthorized" instead of bare "401"). (#U9)
- **Performance footer**: Shows model name instead of provider label; displays "? tok/s" when token count is unavailable. (#U10)
- **Extended syntax highlighting**: Added highlighters for C, C++, Java, Ruby, PHP, Bash with corresponding auto-detection. (#U11)

### Testing

- **Property-based BM25 tests**: 7 new `proptest` cases for `tokenize` (length, lowercase, determinism, null-byte safety, no-alphanumeric) and `build_inverted_index`/`retrieve_relevant_chunks`. (#Q8)

### Code Quality

- **`levenshtein_distance`**: Extracted from private `repl.rs` function to `pub(crate)` in `text_util.rs` for reuse across modules. (#Q9)
- **`status_code_text`**: Added helper mapping HTTP status codes to canonical text in `provider/mod.rs`. (#Q10)

## v0.6.0 (unreleased)

### New Features
- **DeepSeek provider**: Full provider support for `deepseek-chat` (V3, streaming + tools) and `deepseek-reasoner` (R1, reasoning extraction, no tools). (#F1)
- **`/ping` command**: Quick connectivity test showing latency and model count for the active provider. (#F2)
- **`/status` command**: Single overview showing provider, mode, model context, token usage, turn count, and session duration. (#U3)
- **`/config edit`**: Opens `$EDITOR` on the config file with automatic reload on exit. (#S3)
- **Per-project system prompt override**: `.rem/system_prompt.md` is checked and preferred over the default when present. (#F4)
- **`UserInteraction` trait**: Replaced dual-closure pattern in tool executor with a clean trait, fixing REPL borrow conflicts. (#B3)

### Bug Fixes
- **web_search ignores configured provider**: Now reads `search_provider`/`search_api_key`/`search_cse_id` from config instead of always passing `None`. (#B1)
- **run_command approval in pipe mode**: Removed unconditional stdin prompt that hung in non-interactive mode. (#B2)
- **Vision image size validation**: Enforces 20 MB limit before reading/encoding images. (#B4)
- **edit_file replaces last occurrence**: Changed `rfind` to `find` for predictable first-match replacement. (#B5)
- **Memory file locking**: Added `fs2::FileExt::lock_exclusive()` for safe concurrent writes. (#B6)
- **Gemini tool call ID collision**: Replaced name-based IDs with `AtomicU32` counter. (#B7)
- **Retry jitter randomization**: Replaced deterministic `DefaultHasher` with `rand::thread_rng()`. (#O5)

### Optimizations
- **Parallel tool execution**: Non-interactive tool calls (read, write, search, git, web) now execute in parallel via `tokio::spawn`. (#O3)
- **Atomic config saves**: Config writes use tmp file + rename for crash safety. (#S4)
- **Lazy index loading** (already implemented): Codebase index loads on first query, not at startup. (#O1)
- **Token estimate caching** (already implemented): Reuses cached estimates instead of re-estimating on every prompt build. (#O2)

### Testing
- Blocklist edge cases: unicode, null byte, mixed-case, whitespace normalization, control char stripping
- Vision edge cases: file size limits, MIME type edge extensions
- Memory concurrent access: thread-safety verification
- HistoryManager eviction: token budget, turn cap, empty history
- DeepSeek provider: stream chunk deserialization, reasoning content field

### CI & Tooling
- `cargo audit` and `cargo deny` steps made blocking (no longer `continue-on-error`)
- Justfile: `check-all` target added (check + test + clippy + lint)

## v0.5.0 (unreleased)

### New Features
- **Token-based history sliding window**: `HistoryManager` now tracks estimated token budget per session, dropping oldest turns when the budget is exceeded. Configurable via model context window (~60% reserved for history). (#S4)
- **Extended syntax highlighting**: Added `highlight_python()`, `highlight_go()`, `highlight_json()` with keyword regexes. JSON auto-detection from content. (#U3)
- **Markdown table rendering**: Pipe-delimited tables are detected, column widths calculated, and rendered with aligned borders and header highlighting. (#U2)
- **Markdown task list rendering**: `- [ ]` and `- [x]` list items render with styled unchecked (`○`) and checked (`✓`) symbols. (#U2)
- **Session duration & total token display**: Performance footer now shows cumulative session tokens (`Σ N tok`) and wall-clock session duration (`⟖ Nm`). Stats shown after every turn (not just Plan/verbose mode). (#U6)
- **Session export/import**: `/session export`, `/session export-md`, `/session import` for sharing and backing up sessions (gzipped JSON or Markdown). (#F4)
- **Configurable page threshold**: `page_threshold` config field in `AppConfig` and `PartialConfig`, wired through `pager::init_page_threshold()`. (#F3)
- **Incremental indexing**: `generate_codebase_index()` tracks file mtimes and only reprocesses changed files on re-index. (#F1)
- **Custom `ProviderError` enum**: Typed error variants (Auth, RateLimit, Timeout, ServerError, ParseError, ResponseTooLarge) replacing bare `anyhow::Error` in provider code. (#F2)

### Bug Fixes
- **API key redaction**: `openai.rs` was passing `None` to `parse_api_error()` instead of `Some(ctx.api_key_str())`, risking key leakage in error responses. Fixed. (#P6)
- **SpinnerGuard panic**: Removed `handle.abort()` in `output.rs:62`; the `running` flag now cleanly terminates the spinner task within 80ms.
- **`human_size()` overflow**: Added cap at `9999.9` for very large byte values in `text_util.rs`. (#Q4)

### Code Quality & Optimization
- **`file_icon_for()` simplification**: Replaced unwieldy `ends_with` chains with a clean extension-based `match` using `rsplit('.')` in `types.rs`. (#Q5)
- **`AnthropicDelta.thinking` wired**: Removed `#[allow(dead_code)]` and added `delta.thinking` fallback read in `stream_anthropic_sse()`. (#Q6)
- **`classify_intent_heuristic` extracted**: Broken into `detect_web_intent`, `detect_planning_intent`, `detect_fix_intent` focused helpers. (#Q7)

### UI/UX
- **Startup banner**: Shows provider/model label and mode chip. (#U4)
- **Unknown command suggestions**: Levenshtein-based `did_you_mean` with distance ≤ 2 surfaced in REPL. (#U5)

### CI & Tooling
- **Security scanning**: Added `cargo audit` and `cargo deny` to CI workflow. (#P1)
- **Justfile targets**: Added `audit`, `outdated`, `bench` targets. (#P2)
- **`deny.toml`**: License allowlist and dependency ban configuration.

## v0.4.0

### Features
- Categorized help menu with Docker support
- Goal command checkpointing with pagination
- ProviderError enum for structured error handling
- Tool executor with file editing and git support
- BM25 tokenization optimization
- Gzip-compressed index loading
- Command help paging with tips

### Fixes
- Ctrl+C race condition resolution
- curl installer 404 and release asset pipeline repair
- Various bug fixes across the codebase

### Refactoring
- Modularized indexer architecture
- Command runner architecture implementation
- Centralized gzip compression utilities
- Improved CTRL-C handling and config cache
- Token truncation and command output sanitization
