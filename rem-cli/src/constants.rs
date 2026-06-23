//! Centralized magic constants for the REM CLI.
//! All timeouts, byte limits, retry counts, and other tunable values
//! live here to avoid scattering literals across the codebase.

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

// ── Indexer ─────────────────────────────────────────────────────────────────

/// Maximum file size (bytes) to include in the codebase index.
pub const INDEX_MAX_FILE_BYTES: u64 = 120 * 1024; // 120 KB

/// Target chunk size (bytes) for splitting large files.
pub const INDEX_TARGET_CHUNK_BYTES: usize = 2800;

/// Maximum depth for directory walk during indexing.
pub const INDEX_MAX_DEPTH: usize = 8;

// ── BM25 Retrieval ──────────────────────────────────────────────────────────

pub const BM25_K1: f64 = 1.5;
pub const BM25_B: f64 = 0.75;
pub const BM25_NAME_PATH_BONUS: f64 = 2.0;
pub const BM25_CHUNK_TYPE_BONUS: f64 = 0.5;
pub const BM25_EMBEDDING_BONUS_MULT: f64 = 3.0;
pub const BM25_DEFAULT_TOP_K: usize = 8;
pub const BM25_DEFAULT_MAX_CHARS: usize = 4500;

// ── Context / Prompt ────────────────────────────────────────────────────────

/// Max chars for a single @-referenced file's content.
pub const AT_REF_MAX_CHARS: usize = 8000;

/// Max chars for the project file listing fallback.
pub const PROJECT_LISTING_MAX_CHARS: usize = 6000;

/// Max chars for retrieved context from codebase index.
pub const RETRIEVED_CONTEXT_MAX_CHARS: usize = 4800;

/// Max chars for last-generated-code context injection.
pub const LAST_CODE_MAX_CHARS: usize = 6000;

/// Max chars per file in last-files context.
pub const LAST_FILE_MAX_CHARS: usize = 3000;

/// Max chars for piped stdin input.
pub const PIPE_MAX_CHARS: usize = 12000;

/// Max chat history turns to keep.
pub const MAX_HISTORY_TURNS: usize = 12;

/// Turns between auto-save of session.
pub const AUTO_SAVE_INTERVAL: usize = 5;

/// Max readline history entries.
pub const MAX_HISTORY_ENTRIES: usize = 1000;

// ── Tool Execution ──────────────────────────────────────────────────────────

/// Max rounds in the autonomous tool-calling loop.
pub const MAX_TOOL_ROUNDS: usize = 10;

/// Max chars of tool output to feed back to the LLM.
pub const MAX_TOOL_OUTPUT_CHARS: usize = 2000;

/// Max chars of stdout from a run_command tool.
pub const TOOL_COMMAND_STDOUT_MAX: usize = 2000;

/// Max chars of stderr from a run_command tool.
pub const TOOL_COMMAND_STDERR_MAX: usize = 1000;

/// Max chars per tool result in the follow-up prompt.
pub const TOOL_RESULT_MAX_CHARS: usize = 1500;

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

// ── Embeddings ──────────────────────────────────────────────────────────────

/// Max chars of chunk content to send for embedding.
pub const EMBEDDING_MAX_CHUNK_CHARS: usize = 8000;

/// Batch size for concurrent embedding requests.
pub const EMBEDDING_BATCH_SIZE: usize = 10;

/// Ollama embedding model used.
pub const EMBEDDING_MODEL: &str = "nomic-embed-text";

/// Timeout for embedding API calls.
pub const EMBEDDING_TIMEOUT: Duration = Duration::from_secs(120);

// ── Config ───────────────────────────────────────────────────────────────────

/// Default config values.
pub const DEFAULT_MODEL: &str = "rem-coder:latest";
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
pub const DEFAULT_TIMEOUT_S: u64 = 120;
pub const DEFAULT_MAX_CONTEXT_BYTES: usize = 16_000;
pub const DEFAULT_MODEL_CTX: usize = 4096;

/// Minimum reasonable timeout.
pub const MIN_TIMEOUT_S: u64 = 5;
/// Maximum reasonable timeout.
pub const MAX_TIMEOUT_S: u64 = 600;

/// Minimum reasonable model context window.
pub const MIN_MODEL_CTX: usize = 512;

// ── Themes ──────────────────────────────────────────────────────────────────

pub const DEFAULT_THEME: &str = "GHOST";
pub const DEFAULT_MODE: &str = "CHAT";

// ── Search ──────────────────────────────────────────────────────────────────

pub const DEFAULT_SEARCH_PROVIDER: &str = "ddg";
pub const SEARCH_MAX_RESULTS: usize = 8;
