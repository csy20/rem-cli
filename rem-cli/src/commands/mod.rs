//! REPL slash command handlers and command metadata.
//! Each submodule implements a group of `/`-prefixed commands available
//! in the interactive chat session. The [`CommandInfo`] table provides
//! O(1) name → handler lookup, replacing the previous if-else chain.

pub mod files;
pub mod goal;
pub mod help;
pub mod repl;
pub mod review;
pub mod runner;
pub mod session;
pub mod tools;

use std::collections::HashMap;

/// Metadata about a registered slash command.
#[derive(Clone, Copy)]
pub(crate) struct CommandInfo {
    /// Human-readable description for the dynamic help system.
    pub(crate) description: &'static str,
    /// Usage line displayed in help output (e.g. `"/model <name>"`).
    pub(crate) usage: &'static str,
}

/// O(1) lookup for command metadata by name.
pub(crate) struct CommandRegistry {
    commands: HashMap<&'static str, CommandInfo>,
    entries: Vec<(&'static str, CommandInfo)>,
}

impl CommandRegistry {
    pub fn new(entries: &[(&'static str, CommandInfo)]) -> Self {
        let mut commands = HashMap::new();
        for &(name, info) in entries {
            commands.insert(name, info);
        }
        Self {
            commands,
            entries: entries.to_vec(),
        }
    }

    /// Returns true if the input is a registered command.
    pub fn is_command(&self, input: &str) -> bool {
        let name = input.split(' ').next().unwrap_or(input);
        self.commands.contains_key(name)
    }

    /// Returns the command name and argument parts.
    pub fn parse<'a>(&self, input: &'a str) -> (&'a str, &'a str) {
        if let Some(pos) = input.find(' ') {
            (&input[..pos], input[pos + 1..].trim())
        } else {
            (input, "")
        }
    }

    /// Prints formatted help text for all registered commands.
    pub fn print_help(&self) {
        let t = crate::ui::theme::active();
        println!("{}", crate::ui::theme::paint_rail_empty(&t));
        println!("{}", crate::ui::theme::paint_rail_header(&t, "COMMANDS"));
        let mut seen_descriptions: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for &(name, info) in &self.entries {
            if !name.starts_with('/') || !seen_descriptions.insert(info.description) {
                continue;
            }
            println!(
                "{}",
                crate::ui::theme::paint_help_line(&t, info.usage, info.description)
            );
        }
        println!("{}", crate::ui::theme::paint_rail_empty(&t));
        println!("{}", crate::ui::theme::paint_rail_header(&t, "TIPS"));
        println!(
            "{}",
            crate::ui::theme::paint_bullet_line(
                &t,
                &[
                    ("text_faint", "use ", false),
                    ("accent", "@<path>", true),
                    ("text_faint", " to include file context: @src/main.rs", false),
                ]
            )
        );
        println!(
            "{}",
            crate::ui::theme::paint_bullet_line(
                &t,
                &[
                    ("text_faint", "use ", false),
                    ("accent", "/mode", true),
                    ("text_faint", " to toggle between chat, code, and plan modes", false),
                ]
            )
        );
        println!(
            "{}",
            crate::ui::theme::paint_bullet_line(
                &t,
                &[
                    ("accent", "/plan", true),
                    (
                        "text_faint",
                        " for analysis first — REM explores codebase before coding",
                        false
                    ),
                ]
            )
        );
        println!(
            "{}",
            crate::ui::theme::paint_rail_bullet(&t, "describe what you want — REM detects intent")
        );
        println!(
            "{}",
            crate::ui::theme::paint_rail_bullet(&t, "multi-file intent and auto-writes after confirmation")
        );
        println!("{}", crate::ui::theme::paint_rail_empty(&t));
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
            },
        ),
        (
            "help",
            CommandInfo {
                description: "Show this help message",
                usage: "help",
            },
        ),
        (
            "exit",
            CommandInfo {
                description: "Exit the REPL",
                usage: "exit",
            },
        ),
        (
            "quit",
            CommandInfo {
                description: "Exit the REPL",
                usage: "quit",
            },
        ),
        (
            "/theme",
            CommandInfo {
                description: "Change the color theme",
                usage: "/theme [name]",
            },
        ),
        (
            "/model",
            CommandInfo {
                description: "Show or change the active model",
                usage: "/model <name>",
            },
        ),
        (
            "/provider",
            CommandInfo {
                description: "Show or change the LLM provider",
                usage: "/provider <name>",
            },
        ),
        (
            "/mode",
            CommandInfo {
                description: "Switch between chat and code mode",
                usage: "/mode",
            },
        ),
        (
            "/plan",
            CommandInfo {
                description: "Switch to plan mode for structured output",
                usage: "/plan",
            },
        ),
        (
            "/clear",
            CommandInfo {
                description: "Clear the chat history",
                usage: "/clear",
            },
        ),
        (
            "/reset",
            CommandInfo {
                description: "Reset the session",
                usage: "/reset",
            },
        ),
        (
            "/why",
            CommandInfo {
                description: "Explain the last response",
                usage: "/why",
            },
        ),
        (
            "/code",
            CommandInfo {
                description: "Show last generated files",
                usage: "/code",
            },
        ),
        (
            "/undo",
            CommandInfo {
                description: "Undo last file write",
                usage: "/undo",
            },
        ),
        (
            "/files",
            CommandInfo {
                description: "List all project files",
                usage: "/files",
            },
        ),
        (
            "/diff",
            CommandInfo {
                description: "Show diff of last changes",
                usage: "/diff",
            },
        ),
        (
            "/apply",
            CommandInfo {
                description: "Apply the last diff (write changed files with backup for undo)",
                usage: "/apply",
            },
        ),
        (
            "/tokens",
            CommandInfo {
                description: "Show token usage statistics",
                usage: "/tokens",
            },
        ),
        (
            "/memory",
            CommandInfo {
                description: "View or update project memory",
                usage: "/memory [key=value]",
            },
        ),
        (
            "/init",
            CommandInfo {
                description: "Initialize project scaffolding",
                usage: "/init",
            },
        ),
        (
            "/config",
            CommandInfo {
                description: "Show or update configuration",
                usage: "/config [key=value]",
            },
        ),
        (
            "/lint",
            CommandInfo {
                description: "Lint the last written files or a specific path",
                usage: "/lint [file]",
            },
        ),
        (
            "/find",
            CommandInfo {
                description: "Search text inside the project",
                usage: "/find <query>",
            },
        ),
        (
            "/write",
            CommandInfo {
                description: "Write content to a file",
                usage: "/write <path>",
            },
        ),
        (
            "/save",
            CommandInfo {
                description: "Save the session or write content to a file",
                usage: "/save [<path>]",
            },
        ),
        (
            "/dir",
            CommandInfo {
                description: "Change the project directory",
                usage: "/dir <path>",
            },
        ),
        (
            "/copy",
            CommandInfo {
                description: "Copy last N files to clipboard",
                usage: "/copy [N]",
            },
        ),
        (
            "/resume",
            CommandInfo {
                description: "Resume a saved session",
                usage: "/resume",
            },
        ),
        (
            "/compact",
            CommandInfo {
                description: "Compact the chat context",
                usage: "/compact",
            },
        ),
        (
            "/search",
            CommandInfo {
                description: "Search the web",
                usage: "/search <query>",
            },
        ),
        (
            "/explain",
            CommandInfo {
                description: "Explain the selected code",
                usage: "/explain <code>",
            },
        ),
        (
            "/test",
            CommandInfo {
                description: "Generate tests for the selected code",
                usage: "/test <file>",
            },
        ),
        (
            "/refactor",
            CommandInfo {
                description: "Refactor the selected code",
                usage: "/refactor <file>",
            },
        ),
        (
            "/review",
            CommandInfo {
                description: "Review changes for quality issues",
                usage: "/review",
            },
        ),
        (
            "/goal",
            CommandInfo {
                description: "Run autonomous goal-driven loop",
                usage: "/goal <condition>",
            },
        ),
        (
            "/vision",
            CommandInfo {
                description: "Analyze an image with the LLM",
                usage: "/vision <path>",
            },
        ),
        (
            "/reasoning",
            CommandInfo {
                description: "Configure reasoning/thinking mode",
                usage: "/reasoning [on|off|effort]",
            },
        ),
        (
            "/watch",
            CommandInfo {
                description: "Watch files for changes and auto-retry",
                usage: "/watch",
            },
        ),
    ])
}

pub(crate) use crate::vision::handle_vision;
pub(crate) use files::{auto_write_files, handle_copy, handle_undo, handle_write, print_last_files, prompt_for_path};
pub(crate) use goal::handle_goal;
pub(crate) use help::print_chat_help;
pub(crate) use repl::{
    handle_clear, handle_mode, handle_model, handle_plan, handle_provider, handle_reasoning, handle_reset,
    handle_theme, handle_watch, handle_why,
};
pub(crate) use review::{handle_apply, handle_diff, handle_review};
pub(crate) use session::{
    handle_compact, handle_config, handle_config_set, handle_dir, handle_init, handle_list_files, handle_memory,
    handle_memory_set, handle_resume_session, handle_save_session, handle_tokens,
};
pub(crate) use tools::{
    handle_explain, handle_find, handle_lint_with_fallback, handle_refactor, handle_search, handle_test,
};
