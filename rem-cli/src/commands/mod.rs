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
    /// Whether this command needs an async executor (e.g., calls LLM).
    pub(crate) is_async: bool,
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

    /// Returns true if the command is async.
    pub fn is_async(&self, input: &str) -> bool {
        let name = input.split(' ').next().unwrap_or(input);
        self.commands.get(name).map(|c| c.is_async).unwrap_or(false)
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
        // Sync commands
        ("/help", CommandInfo { is_async: false }),
        ("help", CommandInfo { is_async: false }),
        ("exit", CommandInfo { is_async: false }),
        ("quit", CommandInfo { is_async: false }),
        ("/theme", CommandInfo { is_async: false }),
        ("/model", CommandInfo { is_async: false }),
        ("/provider", CommandInfo { is_async: false }),
        ("/mode", CommandInfo { is_async: false }),
        ("/plan", CommandInfo { is_async: false }),
        ("/clear", CommandInfo { is_async: false }),
        ("/reset", CommandInfo { is_async: false }),
        ("/why", CommandInfo { is_async: false }),
        ("/code", CommandInfo { is_async: false }),
        ("/undo", CommandInfo { is_async: false }),
        ("/files", CommandInfo { is_async: false }),
        ("/diff", CommandInfo { is_async: false }),
        ("/tokens", CommandInfo { is_async: false }),
        ("/memory", CommandInfo { is_async: false }),
        ("/init", CommandInfo { is_async: false }),
        ("/config", CommandInfo { is_async: false }),
        ("/lint", CommandInfo { is_async: false }),
        ("/find", CommandInfo { is_async: false }),
        ("/write", CommandInfo { is_async: false }),
        ("/save", CommandInfo { is_async: false }),
        ("/dir", CommandInfo { is_async: false }),
        ("/copy", CommandInfo { is_async: false }),
        ("/save", CommandInfo { is_async: false }),
        ("/resume", CommandInfo { is_async: false }),
        // Async commands (LLM calls)
        ("/search", CommandInfo { is_async: true }),
        ("/explain", CommandInfo { is_async: true }),
        ("/test", CommandInfo { is_async: true }),
        ("/refactor", CommandInfo { is_async: true }),
        ("/review", CommandInfo { is_async: true }),
        ("/compact", CommandInfo { is_async: true }),
        ("/goal", CommandInfo { is_async: true }),
        ("/vision", CommandInfo { is_async: true }),
        ("/reasoning", CommandInfo { is_async: false }),
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
    handle_reset, handle_theme, handle_why,
};
pub(crate) use review::{handle_diff, handle_review};
pub(crate) use session::{
    handle_compact, handle_config, handle_config_set, handle_dir, handle_init, handle_list_files,
    handle_memory, handle_memory_set, handle_resume_session, handle_save_session, handle_tokens,
};
pub(crate) use tools::{
    handle_explain, handle_find, handle_lint, handle_refactor, handle_search, handle_test,
};
