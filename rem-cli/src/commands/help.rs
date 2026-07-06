//! Help command (`/help`).
//! Prints available slash commands and usage tips to the terminal.
//! Content is auto-generated from the [`CommandRegistry`] to stay in sync.

use crate::commands::registry;

pub(crate) fn print_chat_help() {
    registry().print_help();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_chat_help_does_not_panic() {
        print_chat_help();
    }
}
