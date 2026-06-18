//! REPL slash command handlers and command metadata.
//! Each submodule implements a group of `/`-prefixed commands available
//! in the interactive chat session. The [`CommandInfo`] table provides
//! O(1) name → handler lookup, replacing the previous if-else chain.

pub mod files;
pub mod goal;
pub mod help;
pub mod repl;
pub mod review;
pub mod session;
pub mod tools;

use std::collections::HashMap;

/// Metadata about a registered slash command.
#[derive(Clone, Copy)]
pub(crate) struct CommandInfo {
    /// Human-readable description shown in `/help`.
    #[allow(dead_code)]
    pub(crate) description: &'static str,
}

/// O(1) lookup for command metadata by name.
pub(crate) struct CommandRegistry {
    commands: HashMap<&'static str, CommandInfo>,
}

impl CommandRegistry {
    pub fn new(entries: &[(&'static str, CommandInfo)]) -> Self {
        let mut commands = HashMap::new();
        for &(name, info) in entries {
            commands.insert(name, info);
        }
        Self { commands }
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
}

/// Builds the static command registry.
pub(crate) fn registry() -> CommandRegistry {
    CommandRegistry::new(&[
        (
            "/help",
            CommandInfo {
                description: "Show this help message",
            },
        ),
        (
            "help",
            CommandInfo {
                description: "Show this help message",
            },
        ),
        (
            "exit",
            CommandInfo {
                description: "Exit the REPL",
            },
        ),
        (
            "quit",
            CommandInfo {
                description: "Exit the REPL",
            },
        ),
        (
            "/theme",
            CommandInfo {
                description: "Change the color theme",
            },
        ),
        (
            "/model",
            CommandInfo {
                description: "Show or change the active model",
            },
        ),
        (
            "/provider",
            CommandInfo {
                description: "Show or change the LLM provider",
            },
        ),
        (
            "/mode",
            CommandInfo {
                description: "Switch between chat and code mode",
            },
        ),
        (
            "/plan",
            CommandInfo {
                description: "Switch to plan mode for structured output",
            },
        ),
        (
            "/clear",
            CommandInfo {
                description: "Clear the chat history",
            },
        ),
        (
            "/reset",
            CommandInfo {
                description: "Reset the session",
            },
        ),
        (
            "/why",
            CommandInfo {
                description: "Explain the last response",
            },
        ),
        (
            "/code",
            CommandInfo {
                description: "Show last generated files",
            },
        ),
        (
            "/undo",
            CommandInfo {
                description: "Undo last file write",
            },
        ),
        (
            "/files",
            CommandInfo {
                description: "List all project files",
            },
        ),
        (
            "/diff",
            CommandInfo {
                description: "Show diff of last changes",
            },
        ),
        (
            "/tokens",
            CommandInfo {
                description: "Show token usage statistics",
            },
        ),
        (
            "/memory",
            CommandInfo {
                description: "View or update project memory",
            },
        ),
        (
            "/init",
            CommandInfo {
                description: "Initialize project scaffolding",
            },
        ),
        (
            "/config",
            CommandInfo {
                description: "Show or update configuration",
            },
        ),
        (
            "/lint",
            CommandInfo {
                description: "Lint the last written files or a specific path",
            },
        ),
        (
            "/find",
            CommandInfo {
                description: "Search text inside the project",
            },
        ),
        (
            "/write",
            CommandInfo {
                description: "Write content to a file",
            },
        ),
        (
            "/save",
            CommandInfo {
                description: "Save the session or write content to a file",
            },
        ),
        (
            "/dir",
            CommandInfo {
                description: "Change the project directory",
            },
        ),
        (
            "/copy",
            CommandInfo {
                description: "Copy last N files to clipboard",
            },
        ),
        (
            "/resume",
            CommandInfo {
                description: "Resume a saved session",
            },
        ),
        (
            "/compact",
            CommandInfo {
                description: "Compact the chat context",
            },
        ),
        (
            "/search",
            CommandInfo {
                description: "Search the web",
            },
        ),
        (
            "/explain",
            CommandInfo {
                description: "Explain the selected code",
            },
        ),
        (
            "/test",
            CommandInfo {
                description: "Generate tests for the selected code",
            },
        ),
        (
            "/refactor",
            CommandInfo {
                description: "Refactor the selected code",
            },
        ),
        (
            "/review",
            CommandInfo {
                description: "Review changes for quality issues",
            },
        ),
        (
            "/goal",
            CommandInfo {
                description: "Run autonomous goal-driven loop",
            },
        ),
        (
            "/vision",
            CommandInfo {
                description: "Analyze an image with the LLM",
            },
        ),
        (
            "/reasoning",
            CommandInfo {
                description: "Configure reasoning/thinking mode",
            },
        ),
        (
            "/watch",
            CommandInfo {
                description: "Watch files for changes and auto-retry",
            },
        ),
    ])
}

pub(crate) use crate::vision::handle_vision;
pub(crate) use files::{
    auto_write_files, handle_copy, handle_undo, handle_write, print_last_files, prompt_for_path,
};
pub(crate) use goal::handle_goal;
pub(crate) use help::print_chat_help;
pub(crate) use repl::{
    handle_clear, handle_mode, handle_model, handle_plan, handle_provider, handle_reasoning,
    handle_reset, handle_theme, handle_watch, handle_why,
};
pub(crate) use review::{handle_diff, handle_review};
pub(crate) use session::{
    handle_compact, handle_config, handle_config_set, handle_dir, handle_init, handle_list_files,
    handle_memory, handle_memory_set, handle_resume_session, handle_save_session, handle_tokens,
};
pub(crate) use tools::{
    handle_explain, handle_find, handle_lint_with_fallback, handle_refactor, handle_search,
    handle_test,
};
