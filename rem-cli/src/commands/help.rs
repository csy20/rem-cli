//! Help command (`/help`).
//! Prints available slash commands and usage tips to the terminal.

use crate::ui;

pub(crate) fn print_chat_help() {
    let t = ui::theme::active();
    println!("{}", ui::theme::paint_rail_empty(&t));
    println!("{}", ui::theme::paint_rail_header(&t, "COMMANDS"));
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/help", "show this help")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/mode", "toggle CHAT → CODE → PLAN")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/plan", "switch to PLAN mode (explore & analyze)")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(
            &t,
            "/model <name>",
            "switch model (e.g. gpt-4, claude-sonnet-4)"
        )
    );
    println!(
        "{}",
        ui::theme::paint_help_line(
            &t,
            "/provider <name>",
            "switch provider: ollama, openai, gemini, anthropic"
        )
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/clear", "reset conversation history")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/explain <code>", "explain what code does")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/test <file>", "generate tests for a file")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/refactor <file>", "suggest refactoring for a file")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/write <path>", "save last code to file")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/save <path>", "same as /write")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/dir <path>", "set project root")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/search <q>", "search the web (DuckDuckGo)")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/code", "show last generated code")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/files", "list project files tree")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/undo", "delete last written files")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/diff", "compare generated vs existing files")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/tokens", "show token usage & context stats")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/config", "view current configuration")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/memory", "view/set project memory (.rem/memory.md)")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/theme [name]", "show or switch color theme")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/init", "auto-generate project memory file")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/compact", "summarize & free context window")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/goal <cond>", "autonomous loop until goal is met")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/copy [N]", "copy last response to clipboard")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/lint [file]", "run linter on generated files")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/review", "AI code review of generated code")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/find <q>", "search text inside the project")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/reset", "full reset — clear history & code cache")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/save", "save current session to .rem/session.json")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/resume", "restore saved session history")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "/why", "show why last intent was chosen")
    );
    println!(
        "{}",
        ui::theme::paint_help_line(&t, "exit / quit", "exit REM")
    );
    println!("{}", ui::theme::paint_rail_empty(&t));
    println!("{}", ui::theme::paint_rail_header(&t, "TIPS"));
    println!(
        "{}",
        ui::theme::paint_bullet_line(
            &t,
            &[
                ("text_faint", "use ", false),
                ("accent", "@<path>", true),
                (
                    "text_faint",
                    " to include file context: @src/main.rs",
                    false
                ),
            ]
        )
    );
    println!(
        "{}",
        ui::theme::paint_bullet_line(
            &t,
            &[
                ("text_faint", "use ", false),
                ("accent", "/mode", true),
                (
                    "text_faint",
                    " to toggle between chat, code, and plan modes",
                    false
                ),
            ]
        )
    );
    println!(
        "{}",
        ui::theme::paint_bullet_line(
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
        ui::theme::paint_rail_bullet(&t, "describe what you want — REM detects intent")
    );
    println!(
        "{}",
        ui::theme::paint_rail_bullet(&t, "multi-file intent and auto-writes after confirmation")
    );
    println!(
        "{}",
        ui::theme::paint_bullet_line(
            &t,
            &[
                ("text_faint", "use ", false),
                ("accent", "/explain", true),
                ("text_faint", " ", false),
                ("accent", "/test", true),
                ("text_faint", " ", false),
                ("accent", "/refactor", true),
                ("text_faint", " for analysis, tests, and refactoring", false),
            ]
        )
    );
    println!(
        "{}",
        ui::theme::paint_bullet_line(
            &t,
            &[
                ("text_faint", "run ", false),
                ("accent", "/init", true),
                (
                    "text_faint",
                    " for persistent project memory across sessions",
                    false
                ),
            ]
        )
    );
    println!(
        "{}",
        ui::theme::paint_bullet_line(
            &t,
            &[
                ("text_faint", "run ", false),
                ("accent", "rem new <name>", true),
                ("text_faint", " to scaffold a new project instantly", false),
            ]
        )
    );
    println!("{}", ui::theme::paint_rail_empty(&t));
}
