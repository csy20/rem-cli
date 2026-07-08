//! REPL slash command handlers and command metadata.
//! Each submodule implements a group of `/`-prefixed commands available
//! in the interactive chat session. The [`CommandInfo`] table provides
//! O(1) name → handler lookup, replacing the previous if-else chain.

pub mod files;
pub mod git;
pub mod goal;
pub mod help;
pub mod repl;
pub mod review;
pub mod runner;
pub mod session;
pub mod tools;

use std::borrow::Cow;
use std::collections::HashMap;

/// Metadata about a registered slash command.
#[derive(Clone, Copy)]
pub(crate) struct CommandInfo {
    /// Human-readable description for the dynamic help system.
    pub(crate) description: &'static str,
    /// Usage line displayed in help output (e.g. `"/model <name>"`).
    pub(crate) usage: &'static str,
    /// Detailed help text with examples (shown by `/help <command>`).
    pub(crate) long_description: &'static str,
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

    /// Builds the help text body (commands list + tips section).
    fn build_help_body(&self, t: &crate::ui::theme::Theme) -> String {
        let mut buf = String::new();
        let push = |buf: &mut String, s: &str| {
            buf.push_str(s);
            buf.push('\n');
        };
        push(&mut buf, &crate::ui::theme::paint_rail_empty(t));
        push(&mut buf, &crate::ui::theme::paint_rail_header(t, "COMMANDS"));
        let mut seen_descriptions: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for &(name, info) in &self.entries {
            if !name.starts_with('/') || !seen_descriptions.insert(info.description) {
                continue;
            }
            push(
                &mut buf,
                &crate::ui::theme::paint_help_line(t, info.usage, info.description),
            );
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
                long_description: "",
            },
        ),
        (
            "help",
            CommandInfo {
                description: "Show this help message",
                usage: "help",
                long_description: "",
            },
        ),
        (
            "exit",
            CommandInfo {
                description: "Exit the REPL",
                usage: "exit",
                long_description: "",
            },
        ),
        (
            "quit",
            CommandInfo {
                description: "Exit the REPL",
                usage: "quit",
                long_description: "",
            },
        ),
        (
            "/theme",
            CommandInfo {
                description: "Change the color theme",
                usage: "/theme [name]",
                long_description: "",
            },
        ),
        (
            "/model",
            CommandInfo {
                description: "Show or change the active model",
                usage: "/model <name>",
                long_description: "",
            },
        ),
        (
            "/provider",
            CommandInfo {
                description: "Show or change the LLM provider",
                usage: "/provider <name>",
                long_description: "",
            },
        ),
        (
            "/mode",
            CommandInfo {
                description: "Switch between chat and code mode",
                usage: "/mode",
                long_description: "",
            },
        ),
        (
            "/plan",
            CommandInfo {
                description: "Switch to plan mode for structured output",
                usage: "/plan",
                long_description: "",
            },
        ),
        (
            "/clear",
            CommandInfo {
                description: "Clear the chat history",
                usage: "/clear",
                long_description: "",
            },
        ),
        (
            "/reset",
            CommandInfo {
                description: "Reset the session",
                usage: "/reset",
                long_description: "",
            },
        ),
        (
            "/why",
            CommandInfo {
                description: "Explain the last response",
                usage: "/why",
                long_description: "",
            },
        ),
        (
            "/code",
            CommandInfo {
                description: "Show last generated files",
                usage: "/code",
                long_description: "",
            },
        ),
        (
            "/undo",
            CommandInfo {
                description: "Undo last file write",
                usage: "/undo",
                long_description: "",
            },
        ),
        (
            "/files",
            CommandInfo {
                description: "List all project files",
                usage: "/files",
                long_description: "",
            },
        ),
        (
            "/diff",
            CommandInfo {
                description: "Show diff of last changes",
                usage: "/diff",
                long_description: "",
            },
        ),
        (
            "/apply",
            CommandInfo {
                description: "Apply the last diff (write changed files with backup for undo)",
                usage: "/apply",
                long_description: "",
            },
        ),
        (
            "/tokens",
            CommandInfo {
                description: "Show token usage statistics",
                usage: "/tokens",
                long_description: "",
            },
        ),
        (
            "/memory",
            CommandInfo {
                description: "View or update project memory",
                usage: "/memory [key=value]",
                long_description: "",
            },
        ),
        (
            "/init",
            CommandInfo {
                description: "Initialize project scaffolding",
                usage: "/init",
                long_description: "",
            },
        ),
        (
            "/config",
            CommandInfo {
                description: "Show or update configuration",
                usage: "/config [key=value]",
                long_description: "",
            },
        ),
        (
            "/lint",
            CommandInfo {
                description: "Lint the last written files or a specific path",
                usage: "/lint [file]",
                long_description: "",
            },
        ),
        (
            "/find",
            CommandInfo {
                description: "Search text inside the project",
                usage: "/find <query>",
                long_description: "",
            },
        ),
        (
            "/write",
            CommandInfo {
                description: "Write content to a file",
                usage: "/write <path>",
                long_description: "",
            },
        ),
        (
            "/save",
            CommandInfo {
                description: "Save the session or write content to a file",
                usage: "/save [<path>]",
                long_description: "",
            },
        ),
        (
            "/dir",
            CommandInfo {
                description: "Change the project directory",
                usage: "/dir <path>",
                long_description: "",
            },
        ),
        (
            "/copy",
            CommandInfo {
                description: "Copy last N files to clipboard",
                usage: "/copy [N]",
                long_description: "",
            },
        ),
        (
            "/resume",
            CommandInfo {
                description: "Resume a saved session",
                usage: "/resume",
                long_description: "",
            },
        ),
        (
            "/compact",
            CommandInfo {
                description: "Compact the chat context",
                usage: "/compact",
                long_description: "",
            },
        ),
        (
            "/compact-dry-run",
            CommandInfo {
                description: "Preview compaction without calling the LLM",
                usage: "/compact-dry-run",
                long_description: "Shows what /compact would summarize: current turn count and first lines of each turn. Does not call the LLM or modify history.",
            },
        ),
        (
            "/session",
            CommandInfo {
                description: "Export or import a session",
                usage: "/session export <path> | /session import <path>",
                long_description: "",
            },
        ),
        (
            "/search",
            CommandInfo {
                description: "Search the web",
                usage: "/search <query>",
                long_description: "",
            },
        ),
        (
            "/explain",
            CommandInfo {
                description: "Explain the selected code",
                usage: "/explain <code>",
                long_description: "",
            },
        ),
        (
            "/test",
            CommandInfo {
                description: "Generate tests for the selected code",
                usage: "/test <file>",
                long_description: "",
            },
        ),
        (
            "/refactor",
            CommandInfo {
                description: "Refactor the selected code",
                usage: "/refactor <file>",
                long_description: "",
            },
        ),
        (
            "/review",
            CommandInfo {
                description: "Review changes for quality issues",
                usage: "/review",
                long_description: "",
            },
        ),
        (
            "/goal",
            CommandInfo {
                description: "Run autonomous goal-driven loop",
                usage: "/goal <condition>",
                long_description: "",
            },
        ),
        (
            "/vision",
            CommandInfo {
                description: "Analyze an image with the LLM",
                usage: "/vision <path>",
                long_description: "",
            },
        ),
        (
            "/reasoning",
            CommandInfo {
                description: "Configure reasoning/thinking mode",
                usage: "/reasoning [on|off|effort]",
                long_description: "",
            },
        ),
        (
            "/watch",
            CommandInfo {
                description: "Watch files for changes and auto-retry",
                usage: "/watch",
                long_description: "",
            },
        ),
        (
            "/reload",
            CommandInfo {
                description: "Reload config and project settings from disk",
                usage: "/reload",
                long_description: "Clears the config cache and re-reads ~/.config/rem-cli/config.toml and .remcli.toml.\nUseful after editing config files without restarting the REPL.",
            },
        ),
        (
            "/models",
            CommandInfo {
                description: "List available models from the active provider",
                usage: "/models",
                long_description: "Fetches and displays all available models from the current provider (e.g. Ollama, OpenAI).\nUse /model <name> to switch to a different model.",
            },
        ),
        (
            "/pull",
            CommandInfo {
                description: "Pull a model from Ollama",
                usage: "/pull <model-name>",
                long_description: "Downloads a model from Ollama's registry. Only works with the Ollama provider.\nExample: /pull llama3.2:3b",
            },
        ),
        (
            "/commit",
            CommandInfo {
                description: "Stage all changes and create a git commit",
                usage: "/commit [message]",
                long_description: "Runs git add -A and git commit in the project directory.\nIf no message is provided, prompts interactively.\nExample: /commit \"fix: resolve type error in parser\"",
            },
        ),
        (
            "/summary",
            CommandInfo {
                description: "Generate and display a session summary",
                usage: "/summary [save-path]",
                long_description: "Uses the LLM to summarize the current conversation covering key decisions,\ncode generated, bugs fixed, and next actions.\nOptionally save to a file: /summary output.txt",
            },
        ),
    ])
}

pub(crate) use crate::vision::handle_vision;
pub(crate) use files::{auto_write_files, handle_copy, handle_undo, handle_write, print_last_files, prompt_for_path};
pub(crate) use git::handle_commit;
pub(crate) use goal::handle_goal;
pub(crate) use help::{print_chat_help, print_command_help};
pub(crate) use repl::{
    handle_clear, handle_list_models, handle_mode, handle_model, handle_plan, handle_provider, handle_pull_model,
    handle_reasoning, handle_reset, handle_theme, handle_watch, handle_why,
};
pub(crate) use review::{handle_apply, handle_diff, handle_review};
pub(crate) use session::{
    handle_compact, handle_compact_dry_run, handle_compact_undo, handle_config, handle_config_set, handle_dir,
    handle_export_session, handle_import_session, handle_init, handle_list_files, handle_list_sessions, handle_memory,
    handle_memory_set, handle_reload, handle_resume_session, handle_save_session, handle_summary, handle_tokens,
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
                },
            ),
            (
                "bar",
                CommandInfo {
                    description: "Bar command",
                    usage: "bar",
                    long_description: "",
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
