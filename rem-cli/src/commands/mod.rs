//! REPL slash command handlers.
//! Each submodule implements a group of `/`-prefixed commands available
//! in the interactive chat session. Re-exports all `pub(crate)` handlers.

pub mod files;
pub mod goal;
pub mod help;
pub mod repl;
pub mod review;
pub mod session;
pub mod tools;

pub(crate) use files::{
    auto_write_files, handle_copy, handle_undo, handle_write, print_last_files, prompt_for_path,
};
pub(crate) use goal::handle_goal;
pub(crate) use help::print_chat_help;
pub(crate) use repl::{
    handle_clear, handle_mode, handle_model, handle_plan, handle_provider, handle_reset,
    handle_theme, handle_why,
};
pub(crate) use review::{handle_diff, handle_review};
pub(crate) use session::{
    handle_compact, handle_config, handle_config_set, handle_dir, handle_init, handle_list_files,
    handle_memory, handle_memory_set, handle_resume_session, handle_save_session, handle_tokens,
};
pub(crate) use tools::{
    handle_explain, handle_find, handle_lint, handle_refactor, handle_search, handle_test,
};
