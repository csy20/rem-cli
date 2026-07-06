//! Help command (`/help`).
//! Prints available slash commands and usage tips to the terminal.
//! Content is auto-generated from the [`CommandRegistry`] to stay in sync.

use crate::commands::registry;

pub(crate) fn print_chat_help() {
    registry().print_help();
}

pub(crate) fn print_command_help(name: &str) {
    registry().print_command_help(name);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_chat_help_does_not_panic() {
        print_chat_help();
    }

    #[test]
    fn print_command_help_known_does_not_panic() {
        print_command_help("/model");
    }

    #[test]
    fn print_command_help_unknown_does_not_panic() {
        print_command_help("/nonexistent");
    }
}
