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
            let msg = ui::theme::paint_success_label(&t, &format!("theme \u{2192} {}", active_theme.name));
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
            println!("{} model: {}", ui::theme::paint_rail_empty(&t), client.model);
        } else {
            client.set_model(new_model.clone());
            cfg.model = new_model;
            let _ = save_config(cfg);
            let rail = ui::theme::paint_rail_empty(&t);
            let msg = ui::theme::paint_success_label(&t, &format!("model \u{2192} {}", client.model));
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
                let msg = ui::theme::paint_success_label(&t, &format!("provider \u{2192} {}", client.kind.as_str()));
                println!("{rail}");
                println!("{rail} {msg}");
                let model_msg = ui::theme::paint_dim(&t, &format!("model: {}", client.model));
                println!("{rail}  {model_msg}");
                println!("{rail}");
            }
            Err(e) => {
                let rail = ui::theme::paint_rail_empty(&t);
                let msg = ui::theme::paint_error_label(&t, &format!("failed to switch provider: {}", e));
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
    let status = ui::theme::paint(&t, mode_key, &format!("switched to {mode_label} mode"), true);
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
    let sub = ui::theme::paint_dim(&t, "explore & plan \u{2014} analyze, propose approach, no code");
    println!("{rail}");
    println!("{rail} {status}");
    println!("{rail}  {sub}");
    println!("{rail}");
}

pub(crate) fn handle_clear(session: &mut ChatSession) {
    let t = ui::theme::active();
    session.history_mgr.history.clear();
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
    session.history_mgr.history.clear();
    session.last_search.clear();
    session.last_tokens = 0;
    session.code_out.last_code.clear();
    session.code_out.last_files.clear();
    session.code_out.last_files_written.clear();
    let rail = ui::theme::paint_rail_empty(&t);
    let msg = ui::theme::paint_success_label(&t, "full reset \u{2014} history, code cache, and results cleared");
    let sub = ui::theme::paint_dim(&t, "(memory preserved \u{2014} use /memory to clear project memory)");
    println!("{rail}");
    println!("{rail} {msg}");
    println!("{rail}   {sub}");
    println!("{rail}");
}

pub(crate) fn handle_reasoning(client: &mut Provider, cfg: &mut AppConfig, tail: Option<&str>) {
    let t = ui::theme::active();
    let rail = ui::theme::paint_rail_empty(&t);
    if let Some(args) = tail {
        let args = args.trim().to_lowercase();
        match args.as_str() {
            "on" | "enable" => {
                client.reasoning_config.enabled = true;
                cfg.reasoning_effort = Some(client.reasoning_config.effort.as_str().to_string());
                let msg = ui::theme::paint_success_label(&t, "reasoning enabled");
                println!("{rail}");
                println!("{rail} {msg}");
                println!("{rail}");
            }
            "off" | "disable" => {
                client.reasoning_config.enabled = false;
                cfg.reasoning_effort = None;
                let msg = ui::theme::paint_success_label(&t, "reasoning disabled");
                println!("{rail}");
                println!("{rail} {msg}");
                println!("{rail}");
            }
            "low" | "medium" | "high" => {
                let effort = crate::reasoning::ReasoningEffort::from_str(&args);
                client.reasoning_config.effort = effort;
                client.reasoning_config.enabled = true;
                cfg.reasoning_effort = Some(effort.as_str().to_string());
                let msg = ui::theme::paint_success_label(&t, &format!("reasoning effort \u{2192} {}", effort.as_str()));
                println!("{rail}");
                println!("{rail} {msg}");
                println!("{rail}");
            }
            "show" => {
                client.reasoning_config.show_reasoning = true;
                let msg = ui::theme::paint_success_label(&t, "showing reasoning trace");
                println!("{rail}");
                println!("{rail} {msg}");
                println!("{rail}");
            }
            "hide" => {
                client.reasoning_config.show_reasoning = false;
                let msg = ui::theme::paint_success_label(&t, "hiding reasoning trace");
                println!("{rail}");
                println!("{rail} {msg}");
                println!("{rail}");
            }
            _ if args.starts_with("budget ") => {
                if let Ok(n) = args.trim_start_matches("budget ").parse::<u32>() {
                    client.reasoning_config.thinking_budget = n;
                    cfg.thinking_budget = Some(n);
                    let msg = ui::theme::paint_success_label(&t, &format!("thinking budget \u{2192} {} tokens", n));
                    println!("{rail}");
                    println!("{rail} {msg}");
                    println!("{rail}");
                } else {
                    let msg = ui::theme::paint_error_label(&t, "invalid budget — usage: /reasoning budget <tokens>");
                    println!("{rail} {msg}");
                    println!("{rail}");
                }
            }
            _ => {
                let msg =
                    ui::theme::paint_warning(&t, "usage: /reasoning [on|off|low|medium|high|show|hide|budget <n>]");
                println!("{rail}");
                println!("{rail} {msg}");
                println!("{rail}");
            }
        }
    } else {
        // Toggle
        client.reasoning_config.enabled = !client.reasoning_config.enabled;
        if client.reasoning_config.enabled {
            cfg.reasoning_effort = Some(client.reasoning_config.effort.as_str().to_string());
            let msg = ui::theme::paint_success_label(&t, "reasoning enabled");
            let detail = ui::theme::paint_dim(
                &t,
                &format!(
                    "effort: {}  budget: {} tokens  show_trace: {}",
                    client.reasoning_config.effort.as_str(),
                    client.reasoning_config.thinking_budget,
                    client.reasoning_config.show_reasoning,
                ),
            );
            println!("{rail}");
            println!("{rail} {msg}");
            println!("{rail}  {detail}");
            println!("{rail}");
        } else {
            cfg.reasoning_effort = None;
            let msg = ui::theme::paint_success_label(&t, "reasoning disabled");
            println!("{rail}");
            println!("{rail} {msg}");
            println!("{rail}");
        }
    }
    let _ = save_config(cfg);
}

pub(crate) fn handle_watch(session: &ChatSession) {
    let t = ui::theme::active();
    let dir = session
        .ctx
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let rail = ui::theme::paint_rail_empty(&t);
    println!("{rail}");
    match crate::watcher::watch_and_reindex(&dir) {
        Ok(_tx) => {
            let msg = ui::theme::paint_success_label(&t, "file watcher started");
            let sub = ui::theme::paint_dim(&t, &format!("watching {} for changes", dir.display()));
            println!("{rail} {msg}");
            println!("{rail}  {sub}");
            println!("{rail}");
        }
        Err(e) => {
            let msg = ui::theme::paint_error_label(&t, &format!("failed to start file watcher: {}", e));
            println!("{rail} {msg}");
            println!("{rail}");
        }
    }
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
    let debug_fix = ui::theme::paint_dim(&t, &format!("  fix_window={fix_hit}  is_question={is_q}"));
    println!("{rail}");
    println!("{rail} {intent_label} {intent_val}");
    println!("{rail} {input_label} {input_val}");
    println!("{rail} {debug_intent}");
    println!("{rail} {debug_fix}");
    println!("{rail}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::RunMode;
    use crate::provider::ProviderKind;
    use crate::search::SearchResult;

    fn make_session() -> ChatSession {
        ChatSession::new("test", None).unwrap()
    }

    fn make_cfg() -> AppConfig {
        AppConfig::default()
    }

    #[test]
    fn test_handle_clear_clears_history() {
        let mut session = make_session();
        session.history_mgr.history.push(("hi".into(), "hello".into()));
        session.last_search.push(SearchResult {
            title: "test".into(),
            snippet: "snippet".into(),
            url: "http://example.com".into(),
        });
        session.last_tokens = 42;

        handle_clear(&mut session);

        assert!(session.history_mgr.history.is_empty());
        assert!(session.last_search.is_empty());
        assert_eq!(session.last_tokens, 0);
    }

    #[test]
    fn test_handle_reset_clears_all() {
        let mut session = make_session();
        session.history_mgr.history.push(("hi".into(), "hello".into()));
        session.code_out.last_code = "fn main() {}".into();
        session.last_tokens = 42;

        handle_reset(&mut session);

        assert!(session.history_mgr.history.is_empty());
        assert!(session.last_search.is_empty());
        assert!(session.code_out.last_code.is_empty());
        assert!(session.code_out.last_files.is_empty());
        assert!(session.code_out.last_files_written.is_empty());
        assert_eq!(session.last_tokens, 0);
    }

    #[test]
    fn test_handle_mode_switches_from_chat_to_code() {
        let mut session = make_session();
        let mut cfg = make_cfg();
        assert_eq!(session.mode, RunMode::Chat);

        handle_mode(&mut session, &mut cfg);

        assert_eq!(session.mode, RunMode::Code);
        assert_eq!(cfg.mode, "CODE");
    }

    #[test]
    fn test_handle_mode_switches_from_code_to_plan() {
        let mut session = make_session();
        session.mode = RunMode::Code;
        let mut cfg = make_cfg();

        handle_mode(&mut session, &mut cfg);

        assert_eq!(session.mode, RunMode::Plan);
        assert_eq!(cfg.mode, "PLAN");
    }

    #[test]
    fn test_handle_mode_switches_from_plan_to_chat() {
        let mut session = make_session();
        session.mode = RunMode::Plan;
        let mut cfg = make_cfg();

        handle_mode(&mut session, &mut cfg);

        assert_eq!(session.mode, RunMode::Chat);
        assert_eq!(cfg.mode, "CHAT");
    }

    #[test]
    fn test_handle_plan_sets_plan_mode() {
        let mut session = make_session();
        let mut cfg = make_cfg();
        session.mode = RunMode::Code;

        handle_plan(&mut session, &mut cfg);

        assert_eq!(session.mode, RunMode::Plan);
        assert_eq!(cfg.mode, "PLAN");
    }

    #[test]
    fn test_handle_reasoning_enable() {
        let mut client = Provider::new(
            ProviderKind::Ollama,
            "http://localhost:11434".into(),
            "test".into(),
            30,
            String::new(),
            None,
            4096,
        );
        let mut cfg = make_cfg();
        client.reasoning_config.enabled = false;

        handle_reasoning(&mut client, &mut cfg, Some("on"));

        assert!(client.reasoning_config.enabled);
        assert_eq!(cfg.reasoning_effort, Some("medium".to_string()));
    }

    #[test]
    fn test_handle_reasoning_disable() {
        let mut client = Provider::new(
            ProviderKind::Ollama,
            "http://localhost:11434".into(),
            "test".into(),
            30,
            String::new(),
            None,
            4096,
        );
        let mut cfg = make_cfg();
        client.reasoning_config.enabled = true;

        handle_reasoning(&mut client, &mut cfg, Some("off"));

        assert!(!client.reasoning_config.enabled);
        assert_eq!(cfg.reasoning_effort, None);
    }

    #[test]
    fn test_handle_reasoning_show() {
        let mut client = Provider::new(
            ProviderKind::Ollama,
            "http://localhost:11434".into(),
            "test".into(),
            30,
            String::new(),
            None,
            4096,
        );
        let mut cfg = make_cfg();
        client.reasoning_config.show_reasoning = false;

        handle_reasoning(&mut client, &mut cfg, Some("show"));

        assert!(client.reasoning_config.show_reasoning);
    }

    #[test]
    fn test_handle_reasoning_hide() {
        let mut client = Provider::new(
            ProviderKind::Ollama,
            "http://localhost:11434".into(),
            "test".into(),
            30,
            String::new(),
            None,
            4096,
        );
        let mut cfg = make_cfg();
        client.reasoning_config.show_reasoning = true;

        handle_reasoning(&mut client, &mut cfg, Some("hide"));

        assert!(!client.reasoning_config.show_reasoning);
    }

    #[test]
    fn test_handle_reasoning_budget() {
        let mut client = Provider::new(
            ProviderKind::Ollama,
            "http://localhost:11434".into(),
            "test".into(),
            30,
            String::new(),
            None,
            4096,
        );
        let mut cfg = make_cfg();

        handle_reasoning(&mut client, &mut cfg, Some("budget 16384"));

        assert_eq!(client.reasoning_config.thinking_budget, 16384);
        assert_eq!(cfg.thinking_budget, Some(16384));
    }

    #[test]
    fn test_handle_reasoning_toggle_on() {
        let mut client = Provider::new(
            ProviderKind::Ollama,
            "http://localhost:11434".into(),
            "test".into(),
            30,
            String::new(),
            None,
            4096,
        );
        let mut cfg = make_cfg();
        client.reasoning_config.enabled = false;

        handle_reasoning(&mut client, &mut cfg, None);

        assert!(client.reasoning_config.enabled);
        assert_eq!(cfg.reasoning_effort, Some("medium".to_string()));
    }

    #[test]
    fn test_handle_reasoning_toggle_off() {
        let mut client = Provider::new(
            ProviderKind::Ollama,
            "http://localhost:11434".into(),
            "test".into(),
            30,
            String::new(),
            None,
            4096,
        );
        let mut cfg = make_cfg();
        client.reasoning_config.enabled = true;

        handle_reasoning(&mut client, &mut cfg, None);

        assert!(!client.reasoning_config.enabled);
        assert_eq!(cfg.reasoning_effort, None);
    }

    #[test]
    fn test_handle_why_does_not_panic() {
        let session = make_session();
        handle_why(&session);
    }

    #[test]
    fn test_handle_theme_without_tail_does_not_panic() {
        let mut cfg = make_cfg();
        handle_theme(&mut cfg, None);
    }

    #[test]
    fn test_handle_theme_unknown_does_not_panic() {
        let mut cfg = make_cfg();
        handle_theme(&mut cfg, Some("nonexistent_theme"));
    }
}
