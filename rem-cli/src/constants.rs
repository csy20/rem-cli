//! Centralized magic constants for the REM CLI.
//! All timeouts, byte limits, retry counts, and other tunable values
//! live here to avoid scattering literals across the codebase.
//! Some constants are intentionally unused in code but serve as
//! documented, centrally-tuned defaults — those are marked with #[allow(dead_code)].

use std::time::Duration;

// ── Provider / LLM ──────────────────────────────────────────────────────────

/// Maximum bytes to accumulate from a streaming response before erroring.
pub const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024; // 10 MB

/// Timeout for receiving any single chunk from the stream.
pub const STREAM_CHUNK_TIMEOUT: Duration = Duration::from_secs(60);

/// Max number of retries for transient LLM API errors.
pub const LLM_RETRY_MAX_ATTEMPTS: u32 = 3;

/// Base delay (ms) for exponential backoff between retries.
pub const LLM_RETRY_BASE_DELAY_MS: u64 = 500;

/// Default temperature for completion requests.
pub const DEFAULT_TEMPERATURE: f64 = 0.7;

/// Default max tokens for completion requests.
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Temperature for JSON completion requests.
pub const JSON_TEMPERATURE: f64 = 0.3;

/// Max tokens for JSON completion requests.
pub const JSON_MAX_TOKENS: u32 = 512;

/// Max chars of API error body to include in error messages.
pub const API_ERROR_BODY_MAX_CHARS: usize = 300;

/// Max bytes to read from piped stdin input.
pub const PIPE_INPUT_MAX_BYTES: usize = 512 * 1024; // 512 KB

/// Initial capacity for response string buffers.
pub const INITIAL_BUF_CAPACITY: usize = 4096;

/// Fallback number of threads for Ollama if parallel detection fails.
pub const OLLAMA_NUM_THREADS_FALLBACK: usize = 4;

// ── Indexer ─────────────────────────────────────────────────────────────────

/// Maximum file size (bytes) to include in the codebase index.
pub const INDEX_MAX_FILE_BYTES: u64 = 120 * 1024; // 120 KB

/// Target chunk size (bytes) for splitting large files.
pub const INDEX_TARGET_CHUNK_BYTES: usize = 2800;

/// Maximum depth for directory walk during indexing.
pub const INDEX_MAX_DEPTH: usize = 8;

// ── Context / Prompt ────────────────────────────────────────────────────────

/// Max chat history turns to keep.
pub const MAX_HISTORY_TURNS: usize = 12;

/// Turns between auto-save of session.
pub const AUTO_SAVE_INTERVAL: usize = 5;

/// Max readline history entries.
pub const MAX_HISTORY_ENTRIES: usize = 1000;

// ── Tool Execution ──────────────────────────────────────────────────────────

/// Max rounds in the autonomous tool-calling loop.
pub const MAX_TOOL_ROUNDS: usize = 10;

/// Max chars of stdout from a run_command tool.
pub const TOOL_COMMAND_STDOUT_MAX: usize = 2000;

/// Max chars of stderr from a run_command tool.
pub const TOOL_COMMAND_STDERR_MAX: usize = 1000;

/// Max chars per tool result in the follow-up prompt.
pub const TOOL_RESULT_MAX_CHARS: usize = 1500;

/// Timeout for tool subprocess execution.
pub const TOOL_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

// ── Agentic / Goal ──────────────────────────────────────────────────────────

/// Max iterations for the /goal autonomous loop.
pub const GOAL_MAX_ITERATIONS: usize = 10;

/// Timeout per iteration in the goal loop.
pub const GOAL_ITERATION_TIMEOUT: Duration = Duration::from_secs(120);

/// Max lint/test output chars fed back to the LLM per iteration.
pub const GOAL_TOOL_OUTPUT_MAX_CHARS: usize = 2000;

// ── Search / Find ───────────────────────────────────────────────────────────

/// Default max file bytes for `/find` text search.
pub const FIND_MAX_FILE_BYTES: u64 = 64 * 1024; // 64 KB

/// Default max results for `/find`.
pub const FIND_MAX_RESULTS: usize = 500;

/// Default max depth for `/find`.
pub const FIND_MAX_DEPTH: usize = 8;

/// Max diff lines to display in `/review`.
pub const REVIEW_DIFF_MAX_LINES: usize = 30;

// ── Embeddings ──────────────────────────────────────────────────────────────

/// Max uncompressed bytes for session files beyond which auto-save warns.
pub const MAX_SESSION_FILE_BYTES: usize = 10 * 1024 * 1024; // 10 MB

// ── Config ───────────────────────────────────────────────────────────────────

// ── Search ──────────────────────────────────────────────────────────────────

pub const SEARCH_MAX_RESULTS: usize = 8;

pub(crate) const TOKEN_BUDGET_PER_TURN: usize = 500;

// ── System Prompts ─────────────────────────────────────────────────────────

pub(crate) const DEFAULT_SYSTEM_PROMPT: &str = r##"You are REM, a helpful coding assistant for developers of all levels.

You can chat conversationally OR generate code/files — choose the right mode based on what the user is asking for.

CHAT mode (default):
- User is asking a question, explaining something, greeting you, or having a conversation.
- Reply with a clear, direct text or markdown answer.
- NO code generation, NO file creation, NO JSON. Just answer the question.
- If the user might want code but it's not explicit, ask first: "Would you like me to write code for that?"

CODE mode:
- User has explicitly asked you to create, build, generate, scaffold, fix, refactor, or modify code/files.
- Generate complete, runnable files with clear file paths.
- Use the [MODE: CODE] marker at the start of your response when generating code.
"##;

pub(crate) const CHAT_SYSTEM_PROMPT_CONVERSATIONAL: &str = r##"You are REM, a helpful coding assistant in conversation mode.

[MODE: CHAT]
RULES — follow strictly:
1. The user is chatting, asking a question, greeting you, or making conversation.
2. Reply with a clear, direct text or markdown answer. BE CONCISE.
3. NO code generation. NO file creation. NO multi-file format. NO JSON.
4. If the user might want code but didn't explicitly ask, ASK FIRST: "Would you like me to write code for that?"
5. If the user asks "how would you...", "what's the best way...", "should I use X or Y" — give a plan with trade-offs, but NO code.
6. If you need current info (versions, docs), briefly suggest: "/search <query>". Never guess.
7. Keep it short. The user is a developer.
"##;

pub(crate) const CHAT_SYSTEM_PROMPT_CODE: &str = r##"You are REM, a coding assistant in code generation mode.

[MODE: CODE]
RULES — follow strictly:
1. The user explicitly asked for code. Generate complete, runnable files.
2. First, give a 1-line summary of what you'll create.
3. Then output files using the multi-file format below.
4. Keep explanations minimal. Focus on working code.

=== MULTI-FILE FORMAT ===
Each file MUST have its own ### heading with the full path, then a code fence.

### path/to/file.html
```html
<file content here>
```

### path/to/file.css
```css
<file content here>
```

Always provide complete, runnable code. Do NOT use JSON format — use the multi-file format above.
"##;

pub(crate) const CHAT_SYSTEM_PROMPT_PLAN: &str = r##"You are REM, a coding assistant in planning mode.

[MODE: PLAN]
RULES — follow strictly:
1. The user wants a strategic plan before any code is written.
2. FIRST: analyze the request and context. What needs to be built/fixed?
3. SECOND: explore the codebase — mention relevant files and patterns you see.
4. THIRD: propose an approach with alternatives and trade-offs.
5. FOURTH: recommend a concrete next step.
6. DO NOT generate any code. DO NOT output files. NO code fences. NO JSON.
7. Respond using this exact structured format:

## Analysis
<what needs to be done, context, requirements>

## Proposed Approach
<your recommended solution with architecture decisions>

## Implementation Plan
### Step 1: <short description>
- **File(s):** <paths>
- **Action:** <what to do>

### Step 2: <short description>
- **File(s):** <paths>
- **Action:** <what to do>

## Alternatives Considered
<briefly mention 1-2 alternatives and why they weren't chosen>

## Trade-offs & Risks
<key trade-offs, risks, and mitigations>

## Recommendation
<concise next step>

8. End with: "Should I proceed with this plan? Type /mode to switch to CODE when ready."
"##;
