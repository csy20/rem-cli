# rem — Coding Assistant CLI

`rem` is a **beginner-focused coding assistant** that runs in your terminal and
works with any LLM provider (Ollama, OpenAI, Anthropic, Gemini, Azure, AWS Bedrock,
OpenRouter, DeepSeek). It is designed for HTML, CSS, terminal basics, and project scaffolding,
with a structured contract so model output is predictable and safe to preview.

The CLI is written in Rust (`rem-cli/`) and supports **8 LLM providers**, **34+ slash
commands**, **BM25 codebase indexing**, **autonomous goal loops**, and a polished
terminal UI with **6 color themes**.

## Install

### One-line installer (Linux / macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/csy20/rem-cli/main/install.sh | bash
```

Supported platforms: `x86_64` / `aarch64` on Linux and macOS (Apple Silicon included).

### Docker

```bash
# Build the image
cd rem-cli && docker build -t rem-cli .

# Run with Ollama sidecar
docker-compose up -d
docker-compose exec rem ask "create a basic html page"
```

### Build from source

```bash
cd rem-cli
cargo build --release
./target/release/rem ask "create a basic html page with linked css"
```

Requires **Rust 1.78+** and a running Ollama instance (or API key for other providers).

## Quick start

```bash
# One-shot coding question
rem ask "create a simple html page with a header and footer"

# Safe terminal-command explanation
rem explain "rm -rf build"

# Patch preview for a file
rem patch --file index.html --task "add a navigation bar"

# Scaffold a new project
rem new my-site --project-type landing

# Interactive chat (REPL)
rem chat
```

## Pipe mode (non-interactive)

```bash
# Analyze logs
tail -100 app.log | rem

# Review git changes
git diff main | rem

# Check error output
cargo build 2>&1 | rem
```

## Features

### 🎯 Core

- **8 LLM providers**: Ollama (default), OpenAI, Anthropic Claude, Google Gemini,
  Azure OpenAI, AWS Bedrock, OpenRouter, DeepSeek
- **Three interaction modes**: CHAT (conversation), CODE (generation), PLAN (analysis)
- **Streaming responses**: tokens appear as they're generated
- **Pipe mode**: `cat error.log | rem` — non-interactive stdin processing
- **@ file references**: `fix the bug in @src/utils/auth.js` — inject file/dir context
- **Persistent project memory**: `.rem/memory.md` with auto-generation per project type

### 🔧 Code & Files

- **Autonomous goal loop** (`/goal`): iteratively generates code, runs lint/tests,
  feeds results back to the LLM until goal met — with checkpointing and circuit breakers
- **Multi-file generation**: auto-detects `### path/to/file` headings in LLM output
- **Atomic file writes**: temp + rename pattern with backup for undo
- **Edit tool** (`edit_file`): replaces the first occurrence of `old_string` when
  multiple matches exist
- **Undo stack**: `/undo` reverts file creates/overwrites, `/undo N` for N levels

### 🔍 Search & Context

- **Codebase indexing**: pure-Rust BM25 retrieval with incremental updates
- **Web search**: DuckDuckGo, Google, Bing integration
- **Filesystem search**: `/find <query>` with gitignore-awareness and regex mode
- **Relevant project context**: auto-injected from BM25 index on each query

### 🎨 Terminal UI

- **6 built-in color themes**: GHOST (dark), PHOSPHOR (green), MIST (blue),
  PAPER (light), SAKURA (pink), EMBER (orange), CONTRAST (high-contrast)
- **Custom themes**: TOML-based theme files in `~/.config/rem-cli/themes/`
- **Syntax highlighting**: language-aware code highlighting in terminal output
- **Dynamic terminal width**: adapts to terminal resize (SIGWINCH)
- **Grouped `/help`**: commands organized by category (Session, Code, Tools, Project, Model, System)

### 🛡️ Safety

- **Command blocklist**: dangerous patterns (rm -rf /, dd, chmod 777, pipe to shell)
  are flagged and blocked
- **Path traversal prevention**: `resolve_safe_path` ensures writes stay within workspace
- **API key redaction**: sensitive keys are redacted from error messages
- **Non-shell execution**: tool commands use safe subprocess APIs with timeouts

## Slash commands (34+)

### Session
| Command | Description |
|---|---|
| `/help` | Show help with category groups |
| `/clear` | Clear chat history |
| `/reset` | Full reset — history, code cache, search |
| `/mode` | Toggle CHAT → CODE → PLAN |
| `/plan` | Switch directly to PLAN mode |
| `/save [path]` | Save session or write to file |
| `/resume` | Restore saved session |
| `/session export/import` | Export/import session data |
| `/compact` | Summarize & free context window |
| `/compact-dry-run` | Preview compaction |
| `/why` | Show intent classification reasoning |
| `/summary` | Generate session summary via LLM |
| `/ping` | Test provider connectivity & latency |
| `/status` | Show session overview (tokens, time, turns, index) |

### Code
| Command | Description |
|---|---|
| `/write <path>` | Save last generated code to file |
| `/code` | Show last generated files |
| `/undo [N]` | Undo last N file writes |
| `/diff` | Compare generated vs existing files |
| `/apply` | Apply the last diff |
| `/copy [N]` | Copy last N responses to clipboard |
| `/goal <condition>` | Autonomous loop until condition met |
| `/vision <path>` | Analyze an image with the LLM |

### Tools
| Command | Description |
|---|---|
| `/search <query>` | Search the web |
| `/explain <code>` | Explain what code does |
| `/test <file>` | Generate tests for a file |
| `/refactor <file>` | Suggest refactoring improvements |
| `/review` | AI code review of generated code |
| `/lint [file]` | Run linter on generated files |
| `/find <query>` | Search text inside the project |

### Project
| Command | Description |
|---|---|
| `/dir <path>` | Set project workspace directory |
| `/files` | List project file tree |
| `/memory [key=val]` | View or update project memory |
| `/config [key=val]` | View or update configuration |
| `/config edit` | Open config in `$EDITOR` with auto-reload |
| `/init` | Auto-generate `.rem/memory.md` |
| `/reload` | Reload config from disk |

### Model
| Command | Description |
|---|---|
| `/model <name>` | Show or change the active model |
| `/provider <name>` | Switch LLM provider |
| `/models` | List available models |
| `/pull <model>` | Pull a model via Ollama |
| `/reasoning [on/off/effort]` | Configure reasoning/thinking mode |

### System
| Command | Description |
|---|---|
| `/theme [name]` | Change the color theme |
| `/tokens` | Show token usage & context stats |
| `/watch` | Watch files for changes and auto-retry |
| `/commit [msg]` | Stage all changes and git commit |

## @ File references

```
rem> explain the authentication flow in @src/auth.rs
rem> what tests cover @tests/integration/ ?
rem> fix the bug — @src/utils.ts handles this poorly
```

- **Files**: contents injected (up to 8000 chars)
- **Directories**: file listing with entry count injected
- **HTTP URLs**: ignored (pass through without injection)

## Persistent project memory

REM stores project conventions in `.rem/memory.md`:

```bash
# Auto-generate from project structure
rem> /init

# View current memory
rem> /memory

# Add a convention
rem> /memory add Always use async/await, never .then()
```

The memory file is loaded automatically at the start of every session,
and language-specific guidance is injected into the system prompt.

## Configuration

Copy `rem-cli/.remcli.toml.example` to `.remcli.toml` in your project root, or
create `~/.config/rem-cli/config.toml`.

```toml
model = "qwen2.5-coder:1.5b"
ollama_url = "http://localhost:11434"
timeout_s = 120
max_context_bytes = 16000
workspace_dir = "."
mode = "CHAT"

# For remote providers:
# api_key = "sk-..."
# provider = "openai"
```

### Config layers (lowest to highest priority):
1. Built-in defaults
2. Global config: `~/.config/rem-cli/config.toml`
3. Local config: `.remcli.toml` in project root
4. CLI arguments (e.g. `--model`, `--provider`)

## Requirements

- **Ollama** (for local models): `ollama pull qwen2.5-coder:1.5b`
- Or an API key for: OpenAI, Anthropic, Gemini, Azure, OpenRouter, DeepSeek

For low-RAM machines (4-6 GB):

```bash
export OLLAMA_FLASH_ATTENTION=1
export OLLAMA_KV_CACHE_TYPE=q8_0
export OLLAMA_MMAP=1
export OLLAMA_MAX_LOADED_MODELS=1
```

### Provider API keys

Set the corresponding environment variable or add to config:

| Provider | Env var | Config key |
|---|---|---|
| OpenAI | `OPENAI_API_KEY` | `api_key` |
| Anthropic | `ANTHROPIC_API_KEY` | `api_key` |
| Gemini | `GEMINI_API_KEY` | `api_key` |
| Azure | `AZURE_OPENAI_API_KEY` | `api_key` |
| OpenRouter | `OPENROUTER_API_KEY` | `api_key` |
| DeepSeek | `DEEPSEEK_API_KEY` | `api_key` |

## Safety model

- **Dangerous command patterns** are flagged and blocked in output
- **The CLI does not execute shell commands** from LLM output directly
- **Tool execution** uses safe subprocess APIs with configurable timeouts
- **Path traversal** is prevented by `resolve_safe_path` directory checks
- **API keys** are redacted from error messages to prevent leakage

## Troubleshooting

If you see `Ollama request failed: 404`:

- Ensure Ollama is running: `ollama list`
- Run with explicit model: `rem --model qwen2.5-coder:1.5b chat`
- If base URL includes `/api`, the CLI handles it automatically

If you see `Connection refused`:

- Ensure Ollama is running on the expected port (default: 11434)
- For Docker: use `http://ollama:11434` (internal Docker network)

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     CLI Layer (main.rs + cli.rs)            │
│  Argument parsing, config loading, Ctrl+C handling          │
├─────────────────────────────────────────────────────────────┤
│                     REPL Layer (repl.rs)                    │
│  Interactive loop: read input → dispatch commands → LLM     │
├─────────────────────────────────────────────────────────────┤
│                   Command Handlers (commands/)              │
│  34+ slash commands organized by category                   │
├───────────────────┬─────────────────────┬───────────────────┤
│  Provider Layer   │  Indexer Layer      │  Session Layer    │
│  8 LLM providers  │  BM25 retrieval     │  History mgmt     │
│  Streaming +      │  Incremental index  │  Context assembly │
│  Tool calling     │  Chunking           │  Mode switching   │
└───────────────────┴─────────────────────┴───────────────────┘
```

## Development

```bash
cd rem-cli
cargo test              # Run all tests (544+ unit, 18 integration)
cargo clippy            # Lint check (zero warnings target)
cargo build --release   # Release build
just all                # Run all: check, lint, fmt, test, build-release
```

See `AGENTS.md` for detailed code conventions and project structure.

## License

MIT
