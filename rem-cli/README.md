# rem-cli (Rust)

Beginner-focused coding assistant CLI for HTML, CSS, and safe terminal basics.  
Now with 30+ slash commands, persistent project memory, pipe mode, and autonomous agent loop.

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

## Interactive Mode Slash Commands

### Session Management
| Command | Description |
|---|---|
| `/help` | Show all commands |
| `/mode` | Toggle CHAT → CODE → PLAN |
| `/plan` | Switch directly to PLAN mode |
| `/clear` | Reset conversation history |
| `/reset` | Full reset — clear history, code cache, search |
| `/save` | Save session to `.rem/session.json` |
| `/resume` | Restore saved session history |
| `/session export <path>` | Export session as gzipped JSON |
| `/session export-md <path>` | Export session as Markdown |
| `/session import <path>` | Import a previously exported session |
| `/summary [save-path]` | LLM-generated session summary |
| `/compact` | Summarize history to free context window |
| `/compact-dry-run` | Preview compaction without calling LLM |
| `/page` | Re-view last output through system pager |
| `exit` / `quit` | Exit REM |

### Model & Provider
| Command | Description |
|---|---|
| `/model <name>` | Show or switch the active model |
| `/provider <name>` | Show or switch LLM provider |
| `/reasoning [on\|off\|effort]` | Configure reasoning/thinking mode |
| `/models` | List available models from provider |
| `/pull <model>` | Pull a model (Ollama only) |
| `/theme [name]` | Change the color theme |

### Code Operations
| Command | Description |
|---|---|
| `/write <path>` | Save last code to file |
| `/code` | Show last generated code |
| `/undo` | Delete last written files |
| `/diff` | Compare generated vs existing files |
| `/apply` | Write changed files with backup for undo |
| `/copy [N]` | Copy last N files to clipboard |
| `/goal <condition>` | Autonomous loop until condition met |
| `/vision <path>` | Analyze an image with the LLM |
| `/commit [message]` | Stage all and create a git commit |

### Project Context
| Command | Description |
|---|---|
| `/init` | Auto-generate `.rem/memory.md` from project structure |
| `/memory` | View project memory |
| `/memory add <text>` | Append to project memory |
| `/memory clear` | Clear project memory |
| `/dir <path>` | Set project workspace |
| `/files` | List project file tree |
| `/reload` | Reload config and project settings from disk |
| `@path` | Inline file/directory context reference |

### Tools & Analysis
| Command | Description |
|---|---|
| `/explain <code>` | Explain what code does |
| `/test <file>` | Generate tests for a file |
| `/refactor <file>` | Suggest refactoring improvements |
| `/lint [file]` | Run linter on generated files |
| `/review` | AI code review of generated code |
| `/find <query>` | Search text inside the project |
| `/search <query>` | Search the web |
| `/watch` | Watch files for changes and auto-retry |

### System & Info
| Command | Description |
|---|---|
| `/tokens` | Show token usage & context stats |
| `/config [key=value]` | View or update configuration |
| `/why` | Show intent classification reasoning |

### Mode Descriptions

- **CHAT** (green) — Reply in plain text. Ask questions, have conversations. No code generated.
- **CODE** (magenta) — Generate code and files. Create, fix, build. Multi-file format supported.
- **PLAN** (blue) — Explore and plan. Analyze codebase, propose approach with trade-offs. No code generated.

All modes: `rem chat`

Used with:
- Shell analysis (`rem explain`)
- File patching (`rem patch`)
- Project scaffolding (`rem new`)

## Pipe Mode

Pipe data directly into REM for non-interactive analysis:

```bash
# Analyze logs
tail -100 app.log | rem

# Review git changes
git diff main | rem

# Check error output
cargo build 2>&1 | rem
```

## @ File References

Include file or directory context directly in your prompts:

```
rem> explain the authentication flow in @src/auth.rs
rem> what tests cover @tests/integration/ ?
rem> fix the bug — @src/utils.ts handles this poorly
```

Files: contents are injected (up to 8000 chars)  
Directories: file listing with entry counts is injected

## Persistent Project Memory

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

The memory file is loaded automatically at the start of every session. Cross-compatible with Claude Code's `CLAUDE.md` format.

## Requirements

- Rust 1.78+
- Ollama running locally (or OpenAI-compatible provider)
- A local model such as `rem-coder:latest` or `qwen2.5-coder:1.5b`

`rem index` is self-contained (no Python). It produces `.rem/codebase_index.json`
so that relevant code chunks are injected instead of full directory listings.

## Build

```bash
cargo build
```

## Quick start

```bash
cargo run -- ask "create a simple html page with a header and footer"
cargo run -- explain "rm -rf build"
cargo run -- patch --file index.html --task "add a navigation bar"
cargo run -- chat

# if your model name is different
cargo run -- --model qwen2.5-coder:1.5b chat
```

### Interactive mode example

```
rem> /init                # generate project memory
rem> create a responsive landing page with a hero section
rem> /plan                # analyze before coding
rem> how would you architect a user dashboard?
rem> /mode                # switch to CODE
rem> create the layout we planned
rem> /review              # AI code review
rem> /lint index.html     # run linter
rem> /diff                # compare with existing files
rem> /compact             # free context
rem> /goal all tests pass # autonomous loop until done
rem> /save                # persist session
rem> /clear               # fresh start
rem> /tokens              # check usage
```

## Config

Copy `.remcli.toml.example` to `.remcli.toml` in project root or create
`~/.config/rem-cli/config.toml`.

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

## 404 troubleshooting

If you see `Ollama request failed: 404`:

- ensure Ollama is running: `ollama list`
- run CLI with explicit model: `cargo run -- --model rem-coder:latest chat`
- if base URL includes `/api`, this CLI now handles it automatically
