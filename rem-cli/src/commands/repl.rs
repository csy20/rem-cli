//! REPL command handlers extracted from `repl.rs`.
//! These are the inline slash commands that were previously handled directly
//! in the main REPL loop: `/theme`, `/model`, `/provider`, `/mode`, `/plan`,
//! `/clear`, `/reset`, `/why`.

use crate::chat::{ChatSession, RunMode};
use crate::cli::AppConfig;
use crate::config::{build_provider, load_system_prompt, save_config};
use crate::intent::has_creation_intent;
use crate::intent::TaskIntent;
use crate::provider::Provider;
use crate::ui;

pub(crate) fn handle_theme(cfg: &mut AppConfig, tail: Option<&str>) {
    let t = ui::theme::active();
    if let Some(name) = tail {
        let name = name.trim();
        if ui::theme::set_active(name) {
            let active_theme = ui::theme::active();
            cfg.theme = active_theme.name.clone();
            let _ = save_config(cfg);
            let rail = ui::theme::paint_rail_empty(&t);
            let msg = ui::theme::paint_success_label(
                &t,
                &format!("theme \u{2192} {}", active_theme.name),
            );
            println!("{rail}");
            println!("{rail} {msg}");
            println!("{rail}");
        } else {
            let rail = ui::theme::paint_rail_empty(&t);
            let msg = ui::theme::paint_warning(&t, &format!("unknown theme '{}'", name));
            println!("{rail} {msg}");
            println!(
                "{rail} {}",
                ui::theme::paint_dim(&t, "available: GHOST, PHOSPHOR, MIST, EMBER, SAKURA, PAPER")
            );
            println!("{rail}");
        }
    } else {
        let themes = ui::theme::list_names();
        println!("{}", ui::theme::paint_rail_empty(&t));
        println!(
            "{} {}",
            ui::theme::paint_rail_empty(&t),
            ui::theme::paint_bright(&t, "themes")
        );
        println!("{}", ui::theme::paint_rail_empty(&t));
        for name in &themes {
            let preview = ui::theme::by_name(name);
            let is_active = name == &t.name;
            let marker = if is_active { "\u{25C8}" } else { "\u{25C7}" };
            let accent = ui::theme::paint(&preview, "accent", marker, true);
            let label = if is_active {
                ui::theme::paint_bright(&preview, &format!(" {} (active)", name))
            } else {
                ui::theme::paint(&preview, "accent_dim", &format!(" {}", name), false)
            };
            let swatch = ui::theme::paint_on(&preview, "accent", "surface", "  ", false);
            println!("{accent} {label}  {swatch}");
        }
        println!("{}", ui::theme::paint_rail_empty(&t));
        println!(
            "{} {}",
            ui::theme::paint_rail_empty(&t),
            ui::theme::paint_dim(&t, "use /theme <name> to switch")
        );
        println!("{}", ui::theme::paint_rail_empty(&t));
    }
}

pub(crate) fn handle_model(client: &mut Provider, cfg: &mut AppConfig, tail: Option<&str>) {
    let t = ui::theme::active();
    if let Some(new_model) = tail {
        let new_model = new_model.trim().to_string();
        if new_model.is_empty() {
            println!(
                "{} model: {}",
                ui::theme::paint_rail_empty(&t),
                client.model
            );
        } else {
            client.set_model(new_model.clone());
            cfg.model = new_model;
            let _ = save_config(cfg);
            let rail = ui::theme::paint_rail_empty(&t);
            let msg =
                ui::theme::paint_success_label(&t, &format!("model \u{2192} {}", client.model));
            println!("{rail}");
            println!("{rail} {msg}");
            println!("{rail}");
        }
    }
}

pub(crate) fn handle_provider(client: &mut Provider, cfg: &mut AppConfig, tail: Option<&str>) {
    let t = ui::theme::active();
    if let Some(new_provider) = tail {
        let new_provider = new_provider.trim().to_lowercase();
        if new_provider.is_empty() {
            let rail = ui::theme::paint_rail_empty(&t);
            let label = ui::theme::paint_bright(&t, "current provider:");
            let val = ui::theme::paint_dim(&t, client.kind.as_str());
            println!("{rail}");
            println!("{rail} {label} {val}");
            println!("{rail}");
            return;
        }
        let system_prompt = load_system_prompt(cfg.prompts_dir.as_deref());
        match build_provider(cfg, system_prompt) {
            Ok(new_client) => {
                cfg.provider = new_provider;
                let _ = save_config(cfg);
                *client = new_client;
                let rail = ui::theme::paint_rail_empty(&t);
                let msg = ui::theme::paint_success_label(
                    &t,
                    &format!("provider \u{2192} {}", client.kind.as_str()),
                );
                println!("{rail}");
                println!("{rail} {msg}");
                let model_msg = ui::theme::paint_dim(&t, &format!("model: {}", client.model));
                println!("{rail}  {model_msg}");
                println!("{rail}");
            }
            Err(e) => {
                let rail = ui::theme::paint_rail_empty(&t);
                let msg =
                    ui::theme::paint_error_label(&t, &format!("failed to switch provider: {}", e));
                println!("{rail} {msg}");
                println!("{rail}");
            }
        }
    }
}

pub(crate) fn handle_mode(session: &mut ChatSession, cfg: &mut AppConfig) {
    let t = ui::theme::active();
    session.mode = session.mode.toggle();
    let mode_label = session.mode.label();
    cfg.mode = mode_label.to_string();
    let _ = save_config(cfg);
    let mode_key = ui::theme::accent_for_mode(mode_label);
    let hint = match session.mode {
        RunMode::Chat => "reply in plain text \u{2014} ask questions, chat",
        RunMode::Code => "generate code/files \u{2014} create, fix, build",
        RunMode::Plan => "explore & plan \u{2014} analyze, propose approach, no code",
    };
    let rail = ui::theme::paint_rail_empty(&t);
    let status = ui::theme::paint(
        &t,
        mode_key,
        &format!("switched to {mode_label} mode"),
        true,
    );
    let sub = ui::theme::paint_dim(&t, hint);
    println!("{rail}");
    println!("{rail} {status}");
    println!("{rail}  {sub}");
    println!("{rail}");
}

pub(crate) fn handle_plan(session: &mut ChatSession, cfg: &mut AppConfig) {
    let t = ui::theme::active();
    session.mode = RunMode::Plan;
    cfg.mode = "PLAN".to_string();
    let _ = save_config(cfg);
    let rail = ui::theme::paint_rail_empty(&t);
    let status = ui::theme::paint(&t, "accent_info", "switched to PLAN mode", true);
    let sub = ui::theme::paint_dim(
        &t,
        "explore & plan \u{2014} analyze, propose approach, no code",
    );
    println!("{rail}");
    println!("{rail} {status}");
    println!("{rail}  {sub}");
    println!("{rail}");
}

pub(crate) fn handle_clear(session: &mut ChatSession) {
    let t = ui::theme::active();
    session.history.clear();
    session.last_search.clear();
    session.last_tokens = 0;
    let rail = ui::theme::paint_rail_empty(&t);
    let msg = ui::theme::paint_success_label(&t, "conversation cleared");
    println!("{rail}");
    println!("{rail} {msg}");
    println!("{rail}");
}

pub(crate) fn handle_reset(session: &mut ChatSession) {
    let t = ui::theme::active();
    session.history.clear();
    session.last_search.clear();
    session.last_tokens = 0;
    session.last_code.clear();
    session.last_files.clear();
    session.last_files_written.clear();
    let rail = ui::theme::paint_rail_empty(&t);
    let msg = ui::theme::paint_success_label(
        &t,
        "full reset \u{2014} history, code cache, and results cleared",
    );
    let sub = ui::theme::paint_dim(
        &t,
        "(memory preserved \u{2014} use /memory to clear project memory)",
    );
    println!("{rail}");
    println!("{rail} {msg}");
    println!("{rail}   {sub}");
    println!("{rail}");
}

pub(crate) fn handle_why(session: &ChatSession) {
    let t = ui::theme::active();
    let intent_name = match session.last_intent {
        TaskIntent::FastAnswer => "chat/question",
        TaskIntent::Planning => "planning",
        TaskIntent::WebNeeded => "web search needed",
        TaskIntent::CodeAction => "code/file action",
    };
    let rail = ui::theme::paint_rail_empty(&t);
    let intent_label = ui::theme::paint_bright(&t, "last intent:");
    let intent_val = ui::theme::paint_success_label(&t, intent_name);
    let input_label = ui::theme::paint_bright(&t, "last input:");
    let input_val = ui::theme::paint_dim(&t, &format!("\"{}\"", session.last_user_input));
    let create_hit = has_creation_intent(&session.last_user_input);
    let lower_db = session.last_user_input.to_lowercase();
    let fix_hit = lower_db.starts_with("fix ")
        || lower_db.starts_with("refactor ")
        || lower_db.starts_with("rename ")
        || lower_db.starts_with("delete ")
        || lower_db.starts_with("remove ")
        || lower_db.starts_with("optimize ")
        || lower_db.starts_with("update ");
    let is_q = lower_db.starts_with("what ")
        || lower_db.starts_with("how ")
        || lower_db.starts_with("why ")
        || lower_db.starts_with("explain ");
    let debug_intent = ui::theme::paint_dim(&t, &format!("  has_creation_intent={create_hit}"));
    let debug_fix =
        ui::theme::paint_dim(&t, &format!("  fix_window={fix_hit}  is_question={is_q}"));
    println!("{rail}");
    println!("{rail} {intent_label} {intent_val}");
    println!("{rail} {input_label} {input_val}");
    println!("{rail} {debug_intent}");
    println!("{rail} {debug_fix}");
    println!("{rail}");
}
