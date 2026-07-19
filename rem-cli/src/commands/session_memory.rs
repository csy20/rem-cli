use crate::chat::ChatSession;
use crate::ui;
use std::io::IsTerminal;

/// Displays the project memory (`/memory` command).
pub(crate) fn handle_memory(session: &ChatSession) {
    let t = ui::theme::active();
    println!("{}", ui::theme::paint_rail_header(&t, "MEMORY"));
    if session.ctx.project_memory.loaded && !session.ctx.project_memory.content.is_empty() {
        for line in session.ctx.project_memory.content.lines() {
            println!(
                "{} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_dim(&t, line)
            );
        }
    } else {
        println!(
            "{} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(&t, "no project memory yet.")
        );
        println!(
            "{} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(&t, "use /init to generate, or /memory add <text>")
        );
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_dim(&t, "/memory add <text>  /init  /memory clear")
    );
    println!("{}", ui::theme::paint(&t, "accent", "\u{258C}", true));
}

/// Sets or appends to project memory (`/memory set ...`).
pub(crate) fn handle_memory_set(session: &mut ChatSession, args: &str) {
    let t = ui::theme::active();
    if args.eq_ignore_ascii_case("clear") {
        if std::io::stdin().is_terminal() {
            println!(
                "{} {} {}",
                ui::theme::paint_rail_empty(&t),
                ui::theme::paint_warning(&t, "Clear all project memory?"),
                ui::theme::paint_dim(&t, "[y/N]")
            );
            let input = session.readline("rem> ").unwrap_or_default().trim().to_lowercase();
            if input != "y" && input != "yes" {
                return;
            }
        }
        session.ctx.project_memory.content.clear();
        session.ctx.project_memory.loaded = false;
        if let Err(e) = session.ctx.project_memory.save() {
            tracing::warn!("failed to save memory after clear: {}", e);
        }
        println!("{} memory cleared", ui::theme::paint_success_label(&t, "\u{2713}"));
        return;
    }
    if let Some(text) = args.strip_prefix("add ") {
        if let Err(e) = session.ctx.project_memory.append(text) {
            println!("{} failed: {}", ui::theme::paint_error_label(&t, "\u{2717}"), e);
        } else {
            println!(
                "{} appended to memory ({} bytes)",
                ui::theme::paint_success_label(&t, "\u{2713}"),
                text.len()
            );
        }
        return;
    }
    if let Err(e) = session.ctx.project_memory.set(args) {
        println!("{} failed: {}", ui::theme::paint_error_label(&t, "\u{2717}"), e);
    } else {
        println!(
            "{} memory saved ({} bytes)",
            ui::theme::paint_success_label(&t, "\u{2713}"),
            args.len()
        );
    }
}
