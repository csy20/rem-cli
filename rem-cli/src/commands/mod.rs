//! REPL slash command handlers and command metadata.
//! Each submodule implements a group of `/`-prefixed commands available
//! in the interactive chat session. The [`CommandInfo`] table provides
//! O(1) name → handler lookup, replacing the previous if-else chain.

pub mod compare;
pub mod files;
pub mod git;
pub mod goal;
pub mod help;
pub mod plugin;
pub mod prompt;
pub mod repl;
pub mod review;
pub mod runner;
pub mod session;
pub mod tools;

use std::borrow::Cow;
use std::collections::HashMap;

/// Category grouping for `/help` output.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum CommandCategory {
    Session,
    Code,
    Tools,
    Project,
    Model,
    System,
}

/// Metadata about a registered slash command.
#[derive(Clone, Copy)]
pub(crate) struct CommandInfo {
    /// Human-readable description for the dynamic help system.
    pub(crate) description: &'static str,
    /// Usage line displayed in help output (e.g. `"/model <name>"`).
    pub(crate) usage: &'static str,
    /// Detailed help text with examples (shown by `/help <command>`).
    pub(crate) long_description: &'static str,
    /// Category for grouped help display.
    pub(crate) category: CommandCategory,
}

/// O(1) lookup for command metadata by name.
pub(crate) struct CommandRegistry {
    commands: HashMap<&'static str, CommandInfo>,
    entries: Vec<(&'static str, CommandInfo)>,
    command_names: Vec<&'static str>,
}

impl CommandRegistry {
    pub fn new(entries: &[(&'static str, CommandInfo)]) -> Self {
        let mut commands = HashMap::new();
        for &(name, info) in entries {
            commands.insert(name, info);
        }
        let command_names: Vec<&'static str> = entries
            .iter()
            .filter(|(name, _)| name.starts_with('/'))
            .map(|(name, _)| *name)
            .collect();
        Self {
            commands,
            entries: entries.to_vec(),
            command_names,
        }
    }

    /// Returns true if the input is a registered command.
    pub fn is_command(&self, input: &str) -> bool {
        let name = input.split(' ').next().unwrap_or(input);
        self.commands.contains_key(name)
    }

    /// Returns all registered command names.
    pub fn command_names(&self) -> &[&'static str] {
        &self.command_names
    }

    /// Returns the command name and argument parts.
    pub fn parse<'a>(&self, input: &'a str) -> (&'a str, &'a str) {
        if let Some(pos) = input.find(' ') {
            (&input[..pos], input[pos + 1..].trim())
        } else {
            (input, "")
        }
    }

    /// Prints formatted help for a specific command.
    pub fn print_command_help(&self, name: &str) {
        let t = crate::ui::theme::active();
        let lookup: Cow<'_, str> = if name.starts_with('/') {
            Cow::Borrowed(name)
        } else {
            Cow::Owned(format!("/{}", name))
        };
        let info = self.commands.get(lookup.as_ref()).or_else(|| self.commands.get(name));
        match info {
            Some(cmd_info) => {
                let rail = crate::ui::theme::paint_rail_empty(&t);
                println!("{}", rail);
                println!(
                    "{}",
                    crate::ui::theme::paint_help_line(&t, cmd_info.usage, cmd_info.description)
                );
                if !cmd_info.long_description.is_empty() {
                    println!("{}", rail);
                    for line in cmd_info.long_description.lines() {
                        let painted = crate::ui::theme::paint(&t, "text_faint", line, false);
                        println!(
                            "{}  {}",
                            crate::ui::theme::paint(&t, "accent_dim", "\u{258C}", true),
                            painted
                        );
                    }
                }
                println!("{}", rail);
            }
            None => {
                let msg = crate::ui::theme::paint(&t, "error", "Unknown command", false);
                println!("  {msg}: {name}");
            }
        }
    }

    /// Builds the help text body (commands list + tips section), grouped by category.
    fn build_help_body(&self, t: &crate::ui::theme::Theme) -> String {
        use CommandCategory::*;
        let mut buf = String::new();
        let push = |buf: &mut String, s: &str| {
            buf.push_str(s);
            buf.push('\n');
        };
        push(&mut buf, &crate::ui::theme::paint_rail_empty(t));

        let categories = [
            (Session, "SESSION"),
            (Code, "CODE"),
            (Tools, "TOOLS"),
            (Project, "PROJECT"),
            (Model, "MODEL"),
            (System, "SYSTEM"),
        ];
        let mut seen_descriptions: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (cat_id, cat_label) in &categories {
            let mut cat_entries: Vec<&(&str, CommandInfo)> = self
                .entries
                .iter()
                .filter(|(name, info)| name.starts_with('/') && info.category == *cat_id)
                .collect();
            if cat_entries.is_empty() {
                continue;
            }
            cat_entries.sort_by_key(|(name, _)| *name);
            push(&mut buf, &crate::ui::theme::paint_rail_header(t, cat_label));
            for &(_name, info) in &cat_entries {
                if !seen_descriptions.insert(info.description) {
                    continue;
                }
                push(
                    &mut buf,
                    &crate::ui::theme::paint_help_line(t, info.usage, info.description),
                );
            }
        }
        push(&mut buf, &crate::ui::theme::paint_rail_empty(t));
        push(&mut buf, &crate::ui::theme::paint_rail_header(t, "TIPS"));
        push(
            &mut buf,
            &crate::ui::theme::paint_bullet_line(
                t,
                &[
                    ("text_faint", "use ", false),
                    ("accent", "@<path>", true),
                    ("text_faint", " to include file context: @src/main.rs", false),
                ],
            ),
        );
        push(
            &mut buf,
            &crate::ui::theme::paint_bullet_line(
                t,
                &[
                    ("text_faint", "use ", false),
                    ("accent", "/help <command>", true),
                    ("text_faint", " for detailed help: /help /model", false),
                ],
            ),
        );
        push(
            &mut buf,
            &crate::ui::theme::paint_bullet_line(
                t,
                &[
                    ("text_faint", "use ", false),
                    ("accent", "/mode", true),
                    ("text_faint", " to toggle between chat, code, and plan modes", false),
                ],
            ),
        );
        push(
            &mut buf,
            &crate::ui::theme::paint_bullet_line(
                t,
                &[
                    ("accent", "/plan", true),
                    (
                        "text_faint",
                        " for analysis first — REM explores codebase before coding",
                        false,
                    ),
                ],
            ),
        );
        push(
            &mut buf,
            &crate::ui::theme::paint_rail_bullet(t, "describe what you want — REM detects intent"),
        );
        push(
            &mut buf,
            &crate::ui::theme::paint_rail_bullet(t, "multi-file intent and auto-writes after confirmation"),
        );
        push(&mut buf, &crate::ui::theme::paint_rail_empty(t));
        buf
    }

    /// Prints formatted help text for all registered commands.
    pub fn print_help(&self) {
        let t = crate::ui::theme::active();
        let body = self.build_help_body(&t);
        crate::pager::store_output(&body);
        if self.entries.iter().filter(|(name, _)| name.starts_with('/')).count() > 40 {
            crate::pager::maybe_page(&body);
        } else {
            print!("{}", body);
        }
    }
}

/// Builds the static command registry.
pub(crate) fn registry() -> CommandRegistry {
    CommandRegistry::new(&[
        (
            "/help",
            CommandInfo {
                description: "Show this help message",
                usage: "/help",
                long_description: "Shows all available slash commands grouped by category.\nUse /help <command> for detailed help on a specific command.",
                category: CommandCategory::Session,
            },
        ),
        (
            "help",
            CommandInfo {
                description: "Show this help message",
                usage: "help",
                long_description: "Shows all available slash commands grouped by category.\nUse /help <command> for detailed help on a specific command.",
                category: CommandCategory::System,
            },
        ),
        (
            "exit",
            CommandInfo {
                description: "Exit the REPL",
                usage: "exit",
                long_description: "Exits the interactive REPL session. Equivalent to Ctrl+D or typing /quit.",
                category: CommandCategory::System,
            },
        ),
        (
            "quit",
            CommandInfo {
                description: "Exit the REPL",
                usage: "quit",
                long_description: "Exits the interactive REPL session. Equivalent to Ctrl+D or typing exit.",
                category: CommandCategory::System,
            },
        ),
        (
            "/theme",
            CommandInfo {
                description: "Change the color theme",
                usage: "/theme [name]",
                long_description: "Switches the terminal color theme. Available themes:\n  GHOST, PHOSPHOR, MIST, EMBER, SAKURA, PAPER\n\nWithout arguments, lists all themes with a preview of each.",
                category: CommandCategory::System,
            },
        ),
        (
            "/model",
            CommandInfo {
                description: "Show or change the active model",
                usage: "/model <name>",
                long_description: "Switches the active LLM model for the current provider.\nUse /models to see available models from the active provider.\n\nExamples:\n  /model gpt-4o\n  /model claude-sonnet-4-20250514\n  /model rem-coder:latest",
                category: CommandCategory::Model,
            },
        ),
        (
            "/provider",
            CommandInfo {
                description: "Show or change the LLM provider",
                usage: "/provider <name>",
                long_description: "Switches between supported LLM providers:\n  ollama, openai, anthropic, gemini, azure, bedrock, openrouter, deepseek, github, xai\n\nEach provider may require additional configuration in config.toml.\nSee /config for current settings.",
                category: CommandCategory::Model,
            },
        ),
        (
            "/mode",
            CommandInfo {
                description: "Switch between chat and code mode",
                usage: "/mode",
                long_description: "Toggles between:\n  CHAT mode — conversational responses\n  CODE mode — generates runnable code files with /write auto-confirmation\n  PLAN mode — structured analysis output\n\nMode affects how REM interprets your input and formats responses.",
                category: CommandCategory::Session,
            },
        ),
        (
            "/plan",
            CommandInfo {
                description: "Switch to plan mode for structured output",
                usage: "/plan",
                long_description: "Switches to PLAN mode for structured analysis and planning output.\nIn this mode, REM explores the codebase and provides detailed plans\nbefore writing any code. Use /mode to return to CHAT or CODE mode.",
                category: CommandCategory::Session,
            },
        ),
        (
            "/clear",
            CommandInfo {
                description: "Clear the chat history",
                usage: "/clear",
                long_description: "Clears all conversation history from the current session.\nThe session configuration and project context are preserved.\nUse /reset to also clear configuration state.",
                category: CommandCategory::Session,
            },
        ),
        (
            "/reset",
            CommandInfo {
                description: "Reset the session",
                usage: "/reset",
                long_description: "Completely resets the current session: clears history, resets mode to CHAT,\nand refreshes the system prompt. Use /clear to only clear history.",
                category: CommandCategory::Session,
            },
        ),
        (
            "/why",
            CommandInfo {
                description: "Explain the last response",
                usage: "/why",
                long_description: "Asks the LLM to explain its reasoning for the last response.\nUseful for understanding code generation decisions or debugging.",
                category: CommandCategory::Session,
            },
        ),
        (
            "/code",
            CommandInfo {
                description: "Show last generated files",
                usage: "/code",
                long_description: "Displays the list of files generated or modified by the last code action.\nShows file paths and their current status.",
                category: CommandCategory::Code,
            },
        ),
        (
            "/undo",
            CommandInfo {
                description: "Undo last file write",
                usage: "/undo",
                long_description: "Reverts the most recent file write operation.\nBackups are stored in .rem/backups/ for recovery.\nUse multiple times to undo further back.",
                category: CommandCategory::Code,
            },
        ),
        (
            "/files",
            CommandInfo {
                description: "List all project files",
                usage: "/files",
                long_description: "Lists all tracked project files in the current directory.\nShows file sizes and modification timestamps.",
                category: CommandCategory::Project,
            },
        ),
        (
            "/diff",
            CommandInfo {
                description: "Show diff of last changes",
                usage: "/diff",
                long_description: "Shows a git-style diff of the most recent file changes.\nUse /apply to write these changes permanently.",
                category: CommandCategory::Code,
            },
        ),
        (
            "/apply",
            CommandInfo {
                description: "Apply the last diff (write changed files with backup for undo)",
                usage: "/apply",
                long_description: "Writes the changes shown by /diff to disk.\nCreates automatic backups in .rem/backups/ for /undo support.\nConfirms before overwriting existing files.",
                category: CommandCategory::Code,
            },
        ),
        (
            "/tokens",
            CommandInfo {
                description: "Show token usage statistics",
                usage: "/tokens",
                long_description: "Displays total token usage for the current session:\n  - Total tokens used\n  - Estimated cost (when provider pricing is available)\n  - Per-turn breakdown if verbose mode is enabled",
                category: CommandCategory::System,
            },
        ),
        (
            "/memory",
            CommandInfo {
                description: "View or update project memory",
                usage: "/memory [key=value]",
                long_description: "Manages persistent project memory key-value pairs.\nThese are injected into every prompt as context.\n\nUsage:\n  /memory            — list all stored values\n  /memory key=value  — set a value\n  /memory key=       — clear a specific key\n  /memory clear      — clear all memory",
                category: CommandCategory::Project,
            },
        ),
        (
            "/init",
            CommandInfo {
                description: "Initialize project scaffolding",
                usage: "/init",
                long_description: "Scaffolds a new project structure with recommended defaults.\nPrompts for project name, type (rust, python, js, go, etc.),\nand creates the initial directory layout.",
                category: CommandCategory::Project,
            },
        ),
        (
            "/config",
            CommandInfo {
                description: "Show or update configuration",
                usage: "/config [key=value]",
                long_description: "Displays current configuration or updates a specific key.\n\nUsage:\n  /config              — show all settings\n  /config key=value    — set a config value\n  /config provider=anthropic\n\nConfiguration is persisted to ~/.config/rem-cli/config.toml.",
                category: CommandCategory::Project,
            },
        ),
        (
            "/lint",
            CommandInfo {
                description: "Lint the last written files or a specific path",
                usage: "/lint [file]",
                long_description: "Runs the appropriate linter on the specified file or the most recently written files.\nSupported formats: Rust (clippy), Python (ruff), JavaScript (eslint).\nWithout arguments, lints all files written in the last code action.",
                category: CommandCategory::Tools,
            },
        ),
        (
            "/find",
            CommandInfo {
                description: "Search text inside the project",
                usage: "/find <query>",
                long_description: "Full-text search across all project files. Skips .git, node_modules, target, and binary files.\nSupports case-insensitive search and regex patterns.\n\nExamples:\n  /find fn handle_\n  /find -i TODO\n  /find --regex 'fn \\w+'",
                category: CommandCategory::Tools,
            },
        ),
        (
            "/write",
            CommandInfo {
                description: "Write content to a file",
                usage: "/write <path>",
                long_description: "Writes content to a file at the specified path relative to the project directory.\nCreates parent directories automatically. Backups existing files for /undo.\n\nExamples:\n  /write src/main.rs\nthen paste or describe the content to write.",
                category: CommandCategory::Code,
            },
        ),
        (
            "/save",
            CommandInfo {
                description: "Save the session or write content to a file",
                usage: "/save [<path>]",
                long_description: "Without arguments: saves the current session to .rem/session.json.gz for later /resume.\nWith a path: writes the last response content to the specified file.",
                category: CommandCategory::Session,
            },
        ),
        (
            "/dir",
            CommandInfo {
                description: "Change the project directory",
                usage: "/dir <path>",
                long_description: "Changes the working project directory. All file operations (@file references,\n/write, /find, indexing) will use this directory as the root.\n\nExample:\n  /dir ~/projects/my-app",
                category: CommandCategory::Project,
            },
        ),
        (
            "/edit",
            CommandInfo {
                description: "Edit current input in external editor",
                usage: "/edit",
                long_description: "Opens $VISUAL or $EDITOR to write or edit multi-line input.\nSaves and returns the content when the editor exits.",
            category: CommandCategory::Session,
            },
        ),
        (
            "/copy",
            CommandInfo {
                description: "Copy last N files to clipboard",
                usage: "/copy [N]",
                long_description: "Copies the content of the most recently generated files to the system clipboard.\nUse /copy 5 to copy the last 5 files. Uses xclip, xsel, wl-clipboard, or pbcopy.",
            category: CommandCategory::Code,
            },
        ),
        (
            "/resume",
            CommandInfo {
                description: "Resume a saved session",
                usage: "/resume",
                long_description: "Lists saved sessions from .rem/ and prompts to resume one.\nSessions are auto-saved and can be restored with full history.",
            category: CommandCategory::Session,
            },
        ),
        (
            "/compact",
            CommandInfo {
                description: "Compact the chat context",
                usage: "/compact",
                long_description: "Summarizes the conversation history into a compact bullet-point summary\nto save context window space. The original history is backed up to\n.rem/compact_backup.json.gz for recovery via /compact-undo.",
            category: CommandCategory::Session,
            },
        ),
        (
            "/context",
            CommandInfo {
                description: "Show debug context being sent to the model",
                usage: "/context",
                long_description: "Displays the assembled prompt with character/token counts for debugging context injection.\nAlso shows analytics: provider, model, turn count, total tokens, and session duration.",
            category: CommandCategory::Session,
            },
        ),
        (
            "/compact-dry-run",
            CommandInfo {
                description: "Preview compaction without calling the LLM",
                usage: "/compact-dry-run",
                long_description: "Shows what /compact would summarize: current turn count and first lines of each turn. Does not call the LLM or modify history.",
            category: CommandCategory::Session,
            },
        ),
        (
            "/session",
            CommandInfo {
                description: "Manage sessions: export, import, list, analytics",
                usage: "/session export <path> | /session list | /session analytics [path] | /session import <path>",
                long_description: "Advanced session management subcommands:\n  export <path>    — save session to a portable JSON file\n  import <path>    — load a previously exported session\n  list             — show all saved sessions\n  analytics [path] — display token usage and duration stats\n  compact-undo     — restore history from last /compact backup",
            category: CommandCategory::Session,
            },
        ),
        (
            "/search",
            CommandInfo {
                description: "Search the web",
                usage: "/search <query>",
                long_description: "Performs a web search using the configured provider (DuckDuckGo, Google, or Bing).\nResults are displayed as clickable links with snippets.\n\nConfigure the search provider in config.toml:\n  search_provider = \"google\"\n  search_api_key = \"...\"\n  search_cse_id = \"...\"",
            category: CommandCategory::Tools,
            },
        ),
        (
            "/explain",
            CommandInfo {
                description: "Explain the selected code",
                usage: "/explain <code>",
                long_description: "Sends the provided code snippet to the LLM for explanation.\nUseful for understanding unfamiliar code or algorithms.\n\nExample:\n  /explain fn map_err(|e| e.into_inner())",
            category: CommandCategory::Tools,
            },
        ),
        (
            "/test",
            CommandInfo {
                description: "Generate tests for the selected code",
                usage: "/test <file>",
                long_description: "Generates unit tests for the specified file using the LLM.\nAnalyzes the code and produces idiomatic test cases.\n\nExample:\n  /test src/parser.rs",
            category: CommandCategory::Tools,
            },
        ),
        (
            "/refactor",
            CommandInfo {
                description: "Refactor the selected code",
                usage: "/refactor <file>",
                long_description: "Suggests refactoring improvements for the specified file.\nConsiders: code clarity, DRY principle, performance, error handling,\nand idiomatic patterns for the language.",
            category: CommandCategory::Tools,
            },
        ),
        (
            "/review",
            CommandInfo {
                description: "Review changes for quality issues",
                usage: "/review",
                long_description: "Performs a comprehensive code review of the most recent changes.\nChecks for: logic errors, security issues, performance problems,\nstyle violations, and missing edge cases.",
            category: CommandCategory::Tools,
            },
        ),
        (
            "/goal",
            CommandInfo {
                description: "Run autonomous goal-driven loop",
                usage: "/goal <condition>",
                long_description: "Starts an autonomous loop where REM works toward a goal condition.\nThe LLM iteratively plans, executes, and checks progress.\n\nExample:\n  /goal all tests pass\n  /goal implement user authentication",
            category: CommandCategory::Code,
            },
        ),
        (
            "/vision",
            CommandInfo {
                description: "Analyze an image with the LLM",
                usage: "/vision <path>",
                long_description: "Sends an image file to the LLM for visual analysis.\nSupported formats: PNG, JPG, GIF, WebP.\nRequires a provider with vision capabilities (OpenAI, Anthropic, Gemini).",
            category: CommandCategory::Code,
            },
        ),
        (
            "/reasoning",
            CommandInfo {
                description: "Configure reasoning/thinking mode",
                usage: "/reasoning [on|off|effort]",
                long_description: "Controls LLM reasoning and thinking features:\n  on       — enable reasoning mode\n  off      — disable reasoning mode\n  effort   — set reasoning effort (low/medium/high)\n\nSupported by: DeepSeek (deepseek-reasoner), OpenAI (o-series).",
            category: CommandCategory::Model,
            },
        ),
        (
            "/watch",
            CommandInfo {
                description: "Watch files for changes and auto-retry",
                usage: "/watch",
                long_description: "Watches project files for changes and automatically re-runs the last command.\nUseful for test-driven development: edit tests in your editor and\nREM auto-runs them on save.",
            category: CommandCategory::Tools,
            },
        ),
        (
            "/reload",
            CommandInfo {
                description: "Reload config and project settings from disk",
                usage: "/reload",
                long_description: "Clears the config cache and re-reads ~/.config/rem-cli/config.toml and .remcli.toml.\nUseful after editing config files without restarting the REPL.",
            category: CommandCategory::Project,
            },
        ),
        (
            "/ping",
            CommandInfo {
                description: "Test provider connectivity",
                usage: "/ping",
                long_description: "Pings the active provider to check connectivity and latency.\nDisplays response time and available model count.",
            category: CommandCategory::Model,
            },
        ),
        (
            "/models",
            CommandInfo {
                description: "List available models from the active provider",
                usage: "/models",
                long_description: "Fetches and displays all available models from the current provider (e.g. Ollama, OpenAI).\nUse /model <name> to switch to a different model.",
            category: CommandCategory::Model,
            },
        ),
        (
            "/plugin",
            CommandInfo {
                description: "List and run plugins",
                usage: "/plugin list | /plugin <name> [args]",
                long_description: "Extend rem-cli with custom plugin commands.\n  /plugin list        — list all registered plugins\n  /plugin <name>      — run a plugin (with optional args)\n  /plugin help        — show plugin help",
            category: CommandCategory::System,
            },
        ),
        (
            "/pull",
            CommandInfo {
                description: "Pull a model from Ollama",
                usage: "/pull <model-name>",
                long_description: "Downloads a model from Ollama's registry. Only works with the Ollama provider.\nExample: /pull llama3.2:3b",
            category: CommandCategory::Model,
            },
        ),
        (
            "/commit",
            CommandInfo {
                description: "Stage all changes and create a git commit",
                usage: "/commit [message]",
                long_description: "Runs git add -A and git commit in the project directory.\nIf no message is provided, prompts interactively.\nExample: /commit \"fix: resolve type error in parser\"",
            category: CommandCategory::System,
            },
        ),
        (
            "/summary",
            CommandInfo {
                description: "Generate and display a session summary",
                usage: "/summary [save-path]",
                long_description: "Uses the LLM to summarize the current conversation covering key decisions,\ncode generated, bugs fixed, and next actions.\nOptionally save to a file: /summary output.txt",
            category: CommandCategory::Session,
            },
        ),
        (
            "/status",
            CommandInfo {
                description: "Show session and provider status overview",
                usage: "/status",
                long_description: "Displays an overview of the current session: provider, mode, model context window,\ntoken usage, turn count, and session duration.",
            category: CommandCategory::Session,
            },
        ),
        (
            "/page",
            CommandInfo {
                description: "Re-view the last output through a pager",
                usage: "/page",
                long_description: "Pipes the most recent command output through the system pager (less by default).\nUseful when previous output has scrolled off the terminal screen.",
            category: CommandCategory::Session,
            },
        ),
        (
            "/prompt",
            CommandInfo {
                description: "Save, load, list, or delete prompt templates",
                usage: "/prompt save <name> | /prompt load <name> | /prompt list | /prompt delete <name>",
                long_description: "Save the current user input as a reusable prompt template.\nTemplates support {{variable}} placeholders for substitution.\n\nSubcommands:\n  save <name>    — save last input as template\n  save <name>!   — overwrite existing template\n  load <name>    — load and insert a template (prompts for variables)\n  list           — show all saved templates\n  delete <name>  — remove a template\n\nTemplates are stored in .rem/prompts/<name>.md",
            category: CommandCategory::Session,
            },
        ),
        (
            "/compare",
            CommandInfo {
                description: "Compare responses across multiple models",
                usage: "/compare <provider1/model1> <provider2/model2> ...",
                long_description: "Reruns the last user message against one or more model/provider combinations.\nEach model receives the same prompt and system context, and the results are\ndisplayed side by side for comparison.\n\nExamples:\n  /compare anthropic/claude-sonnet-4-20250514 openai/gpt-4o\n  /compare deepseek-chat gemini/gemini-2.0-flash",
            category: CommandCategory::Model,
            },
        ),
        (
            "/git",
            CommandInfo {
                description: "Git workflow commands: status, diff, log, commit",
                usage: "/git status | /git diff [file] | /git log [n]",
                long_description: "Quick git operations without leaving the REPL:\n  /git status    — show working tree status\n  /git diff      — show unstaged diff (optionally for a specific file)\n  /git log [n]   — show last N commits (default 5)",
            category: CommandCategory::System,
            },
        ),
    ])
}

pub(crate) use crate::pager::handle_page;
pub(crate) use crate::vision::handle_vision;
pub(crate) use compare::handle_compare;
pub(crate) use files::{auto_write_files, handle_copy, handle_undo, handle_write, print_last_files, prompt_for_path};
pub(crate) use git::{handle_commit, handle_git_diff, handle_git_log, handle_git_status};
pub(crate) use goal::handle_goal;
pub(crate) use help::{print_chat_help, print_command_help};
pub(crate) use plugin::{handle_plugin, init_plugin_manager};
pub(crate) use prompt::{
    handle_prompt_delete, handle_prompt_list, handle_prompt_load, handle_prompt_save, handle_prompt_save_force,
};
pub(crate) use repl::{
    handle_clear, handle_list_models, handle_mode, handle_model, handle_ping, handle_plan, handle_provider,
    handle_pull_model, handle_reasoning, handle_reset, handle_status, handle_theme, handle_why,
};
pub(crate) use review::{handle_apply, handle_diff, handle_review};
pub(crate) use session::{
    handle_compact, handle_compact_dry_run, handle_compact_undo, handle_config, handle_config_set, handle_context,
    handle_dir, handle_edit, handle_export_session, handle_export_session_md, handle_import_session, handle_init,
    handle_list_files, handle_list_sessions, handle_memory, handle_memory_set, handle_reload, handle_resume_session,
    handle_save_session, handle_session_analytics, handle_summary, handle_tokens,
};
pub(crate) use tools::{
    handle_explain, handle_find, handle_lint_with_fallback, handle_refactor, handle_search, handle_test,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_expected_commands() {
        let reg = registry();
        assert!(reg.is_command("/help"));
        assert!(reg.is_command("help"));
        assert!(reg.is_command("exit"));
        assert!(reg.is_command("quit"));
        assert!(reg.is_command("/model"));
        assert!(reg.is_command("/clear"));
        assert!(reg.is_command("/goal"));
        assert!(reg.is_command("/vision"));
        assert!(reg.is_command("/models"));
        assert!(reg.is_command("/pull"));
        assert!(reg.is_command("/commit"));
        assert!(reg.is_command("/summary"));
    }

    #[test]
    fn registry_rejects_unknown() {
        let reg = registry();
        assert!(!reg.is_command("/nonexistent"));
        assert!(!reg.is_command("foobar"));
        assert!(!reg.is_command(""));
    }

    #[test]
    fn command_names_returns_slash_prefixed() {
        let reg = registry();
        let names = reg.command_names();
        assert!(names.len() > 5);
        assert!(names.iter().all(|n| n.starts_with('/')));
        assert!(names.contains(&"/help"));
        assert!(names.contains(&"/clear"));
    }

    #[test]
    fn command_names_excludes_non_slash() {
        let reg = registry();
        let names = reg.command_names();
        assert!(!names.contains(&"help"));
        assert!(!names.contains(&"exit"));
        assert!(!names.contains(&"quit"));
    }

    #[test]
    fn parse_with_args() {
        let reg = registry();
        let (cmd, args) = reg.parse("/model gpt-4");
        assert_eq!(cmd, "/model");
        assert_eq!(args, "gpt-4");
    }

    #[test]
    fn parse_without_args() {
        let reg = registry();
        let (cmd, args) = reg.parse("/help");
        assert_eq!(cmd, "/help");
        assert_eq!(args, "");
    }

    #[test]
    fn parse_empty() {
        let reg = registry();
        let (cmd, args) = reg.parse("");
        assert_eq!(cmd, "");
        assert_eq!(args, "");
    }

    #[test]
    fn parse_trailing_spaces() {
        let reg = registry();
        let (cmd, args) = reg.parse("/model   gpt-4  ");
        assert_eq!(cmd, "/model");
        assert_eq!(args, "gpt-4");
    }

    #[test]
    fn registry_command_count() {
        let reg = registry();
        let names = reg.command_names();
        // Count all entries starting with '/'
        let slash_count = reg.entries.iter().filter(|(name, _)| name.starts_with('/')).count();
        assert_eq!(names.len(), slash_count);
        assert!(slash_count > 30, "expected at least 30 slash commands");
    }

    #[test]
    fn custom_registry() {
        let entries = [
            (
                "/foo",
                CommandInfo {
                    description: "Foo command",
                    usage: "/foo",
                    long_description: "",
                    category: CommandCategory::System,
                },
            ),
            (
                "bar",
                CommandInfo {
                    description: "Bar command",
                    usage: "bar",
                    long_description: "",
                    category: CommandCategory::System,
                },
            ),
        ];
        let reg = CommandRegistry::new(&entries);
        assert!(reg.is_command("/foo"));
        assert!(reg.is_command("bar"));
        assert!(!reg.is_command("/baz"));
        assert_eq!(reg.command_names(), vec!["/foo"]);
    }
}
