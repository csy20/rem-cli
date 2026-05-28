# rem-cli (Rust)

Beginner-focused coding assistant CLI for HTML, CSS, and safe terminal basics.

## Features

- `rem ask "..."` for coding help
- `rem explain "<command>"` for safe terminal guidance
- `rem patch --file <path> --task "..."` for patch previews
- `rem chat` interactive mode with slash commands
- **Three modes**: CHAT (conversation), CODE (generation), PLAN (analysis)
- Structured JSON model contract for stable parsing
- Built-in command safety filtering

## Interactive Mode Slash Commands

| Command | Description |
|---|---|
| `/help` | Show all commands |
| `/mode` | Toggle CHAT → CODE → PLAN |
| `/plan` | Switch directly to PLAN mode |
| `/clear` | Reset conversation history |
| `/explain <code>` | Explain what code does |
| `/test <file>` | Generate tests for a file |
| `/refactor <file>` | Suggest refactoring improvements |
| `/write <path>` | Save last code to file |
| `/save <path>` | Same as `/write` |
| `/dir <path>` | Set project workspace |
| `/search <query>` | Search the web (DuckDuckGo) |
| `/code` | Show last generated code |
| `/files` | List project file tree |
| `/undo` | Delete last written files |
| `/diff` | Compare generated vs existing files |
| `/tokens` | Show token usage & context stats |
| `/config` | View current configuration |
| `/why` | Show intent classification reasoning |
| `exit` / `quit` | Exit REM |

### Mode Descriptions

- **CHAT** (green) — Reply in plain text. Ask questions, have conversations. No code generated.
- **CODE** (magenta) — Generate code and files. Create, fix, build. Multi-file format supported.
- **PLAN** (blue) — Explore and plan. Analyze codebase, propose approach with trade-offs. No code generated.

All modes: `rem chat`

Used with:
- Shell analysis (`rem explain`)
- File patching (`rem patch`)
- Project scaffolding (`rem new`)

## Requirements

- Rust 1.78+
- Ollama running locally
- A local model such as `rem-coder:latest`

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
cargo run -- --model deepseek-coder:1.3b chat
```

### Interactive mode example

```
rem> create a responsive landing page with a hero section
rem> /plan
rem> how would you architect a user dashboard with real-time updates?
rem> /mode         # switch to CODE
rem> create the dashboard layout we just planned
rem> /clear        # reset conversation
rem> /tokens       # check context usage
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

## Safety model

- Dangerous command patterns are flagged as blocked in output.
- The CLI does not execute shell commands.
- Destructive commands should be replaced by safe previews.

## 404 troubleshooting

If you see `Ollama request failed: 404`:

- ensure Ollama is running: `ollama list`
- run CLI with explicit model: `cargo run -- --model rem-coder:latest chat`
- if base URL includes `/api`, this CLI now handles it automatically
