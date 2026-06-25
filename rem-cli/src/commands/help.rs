//! Help command (`/help`).
//! Prints available slash commands and usage tips to the terminal.
//! Content is auto-generated from the [`CommandRegistry`] to stay in sync.

use crate::commands::registry;

pub(crate) fn print_chat_help() {
    registry().print_help();
}
