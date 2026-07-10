# rem — Coding Assistant CLI

`rem` is a beginner-focused coding assistant that runs in your terminal and talks
to a local Ollama model. It is designed for HTML, CSS, terminal basics, and
small project scaffolding, with a structured contract so the model output is
predictable and safe to preview.

The CLI is written in Rust (`rem-cli/`). The previous Python training
pipeline (data curation, QLoRA fine-tuning, etc.) has been removed — this
repo is now exclusively the `rem` CLI. `rem index` is pure Rust and
generates the retrieval index used by chat/goal for larger projects.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/csy20/rem-cli/main/install.sh | bash
```

This downloads the prebuilt `rem` binary that matches your OS / architecture
and installs it to `~/.local/bin/`. The installer also adds that directory to
`PATH` (via `~/.bashrc` or `~/.zshrc`) if it isn't already.

Supported platforms: `x86_64` / `aarch64` on Linux and macOS (Apple Silicon
included). Pin a version with `VERSION=v0.4.0` if needed.

> **Note:** the one-line installer needs a published GitHub Release. Create one
> by pushing a version tag (`git tag v0.4.0 && git push origin v0.4.0`), which
> triggers the release workflow. Until then, build from source below.

## Build from source

```bash
cd rem-cli
cargo build
cargo run -- ask "create a basic html page with linked css"
```

Rust 1.78+ is required.

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

# Interactive chat
rem chat
```

## Features

- `rem ask "..."` for coding help
- `rem explain "<command>"` for safe terminal guidance
- `rem patch --file <path> --task "..."` for patch previews
- `rem new <name> --project-type <bare|portfolio|landing|blog>` for project scaffolding
- `rem chat` interactive mode with slash commands
- **Three modes**: CHAT (conversation), CODE (generation), PLAN (analysis)
- **Pipe mode**: `cat error.log | rem` — non-interactive stdin processing
- **@ references**: `fix the bug in @src/utils/auth.js` — inject file/dir context
- **Persistent memory**: `.rem/memory.md` survives sessions with `/init` and `/memory`
- **Auto-memory**: `/init` detects project type and generates conventions
- **Autonomous loop**: `/goal "all tests pass"` keeps working until done
- **Session management**: `/save` and `/resume` persist conversations
- Structured JSON model contract for stable parsing
- Built-in command safety filtering

## Slash commands

| Command | Description |
|---|---|
| `/help` | Show all commands |
| `/mode` | Toggle CHAT → CODE → PLAN |
| `/plan` | Switch directly to PLAN mode |
| `/clear` | Reset conversation history |
| `/reset` | Full reset — clear history, code cache, search |
| `/explain <code>` | Explain what code does |
| `/test <file>` | Generate tests for a file |
| `/refactor <file>` | Suggest refactoring improvements |
| `/write <path>` | Save last code to file |
| `/code` | Show last generated code |
| `/init` | Auto-generate `.rem/memory.md` from project structure |
| `/memory` | View project memory |
| `/dir <path>` | Set project workspace |
| `/files` | List project file tree |
| `/search <query>` | Search the web (DuckDuckGo) |
| `/find <query>` | Search text inside the project (skips node_modules, target, .git) |
| `/diff` | Compare generated vs existing files |
| `/review` | AI code review of generated code |
| `/lint [file]` | Run linter on generated files |
| `/tokens` | Show token usage & context stats |
| `/config` | View current configuration |
| `/why` | Show intent classification reasoning |
| `/compact` | Summarize & free context window |
| `/goal <condition>` | Autonomous loop until condition met |
| `/copy [N]` | Copy last response to clipboard |
| `/save` | Save session to `.rem/session.json` |
| `/resume` | Restore saved session history |
| `/undo` | Delete last written files |

## Pipe mode

```bash
# Analyze logs
tail -100 app.log | rem

# Review git changes
git diff main | rem

# Check error output
cargo build 2>&1 | rem
```

## @ File references

```
rem> explain the authentication flow in @src/auth.rs
rem> what tests cover @tests/integration/ ?
rem> fix the bug — @src/utils.ts handles this poorly
```

Files: contents are injected (up to 8000 chars).
Directories: file listing with entry counts is injected.

## Persistent project memory

REM stores project conventions in `.rem/memory.md`:

```bash
# Auto-generate from project structure
rem> /init

# View current memory
rem> /memory

# Add conventions
rem> /memory add Always use async/await, never .then()

# Clear memory
rem> /memory clear
```

The memory file is loaded automatically at the start of every session.

## Config

Copy `rem-cli/.remcli.toml.example` to `.remcli.toml` in your project root or
create `~/.config/rem-cli/config.toml`.

Supported keys:

- `model`
- `ollama_url`
- `timeout_s`
- `max_context_bytes`
- `prompts_dir`
- `workspace_dir`

## Safety model

- Dangerous command patterns are flagged as blocked in output.
- The CLI does not execute shell commands.
- Destructive commands should be replaced by safe previews.

## Requirements

- Ollama running locally
- A model such as `qwen2.5-coder:1.5b` (`ollama pull qwen2.5-coder:1.5b`)

For low-RAM machines (4–6GB), set these env vars before running Ollama:

```bash
export OLLAMA_FLASH_ATTENTION=1    # 30-50% KV cache RAM savings
export OLLAMA_KV_CACHE_TYPE=q8_0   # half precision KV cache
export OLLAMA_MMAP=1               # mmap model load
export OLLAMA_MAX_LOADED_MODELS=1  # keep one model loaded
```

## Troubleshooting

If you see `Ollama request failed: 404`:

- ensure Ollama is running: `ollama list`
- run CLI with explicit model: `rem --model qwen2.5-coder:1.5b chat`
- if base URL includes `/api`, the CLI handles it automatically

See `rem-cli/README.md` for the full reference.
