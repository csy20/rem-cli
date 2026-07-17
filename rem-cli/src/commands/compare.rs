use crate::chat::ChatSession;
use crate::provider::{Provider, ProviderKind};
use crate::ui;
use std::time::Instant;

/// Runs the same prompt against multiple models and compares results (`/compare`).
/// Usage: `/compare <provider1/model1> <provider2/model2> ...`
/// The prompt is taken from the current user input.
pub(crate) async fn handle_compare(session: &mut ChatSession, client: &Provider, models: &str) {
    let t = ui::theme::active();
    let prompt = session.last_user_input.trim().to_string();
    if prompt.is_empty() {
        println!(
            "{} send a message first, then /compare to rerun it against other models",
            ui::theme::paint_warning(&t, "\u{258C}")
        );
        return;
    }
    let model_specs: Vec<&str> = models.split_whitespace().filter(|s| !s.is_empty()).collect();
    if model_specs.is_empty() {
        println!(
            "{} usage: /compare <provider1/model1> <provider2/model2> ...",
            ui::theme::paint_warning(&t, "\u{258C}")
        );
        return;
    }

    println!("{}", ui::theme::paint_rail_header(&t, "COMPARE"));
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_dim(&t, &format!("prompt: {}", truncate(&prompt, 80)))
    );
    println!("{}", ui::theme::paint_rail_empty(&t));

    let system_prompt = client.system_prompt.clone();
    let current_kind = client.kind;
    let mut handles = Vec::new();

    for spec in &model_specs {
        let parts: Vec<&str> = spec.splitn(2, '/').collect();
        let (provider_name, model_name) = if parts.len() == 2 {
            (parts[0], parts[1])
        } else {
            (current_kind.as_str(), parts[0])
        };

        let kind = ProviderKind::from_str(provider_name);
        let base_url = crate::provider::default_base_url(kind);
        let api_key = crate::provider::api_key_env_var(kind).and_then(|var| std::env::var(var).ok());
        let model_ctx = 128_000;
        let sp = system_prompt.clone();
        let compare_client = Provider::new(
            kind,
            base_url,
            model_name.to_string(),
            30,
            sp.clone(),
            api_key,
            model_ctx,
        );

        let p_clone = prompt.clone();
        let mn_clone = model_name.to_string();
        handles.push(tokio::spawn(async move {
            let start = Instant::now();
            let result = compare_client.complete_chat_stream(&p_clone, &sp, "").await;
            let elapsed = start.elapsed();
            (mn_clone, compare_client.provider_label(), result, elapsed)
        }));
    }

    for handle in handles {
        match handle.await {
            Ok((_model, label, result, elapsed)) => {
                let label_painted = ui::theme::paint_bright(&t, &label);
                let dur = ui::theme::paint_dim(&t, &format!("{:.1}s", elapsed.as_secs_f64()));
                println!(
                    "{} {} {}",
                    ui::theme::paint(&t, "accent", "\u{258C}", true),
                    label_painted,
                    dur
                );
                match result {
                    Ok(text) => {
                        let preview = truncate(&text, 500);
                        for line in preview.lines() {
                            println!(
                                "{}   {}",
                                ui::theme::paint(&t, "accent", "\u{258C}", true),
                                ui::theme::paint_dim(&t, line)
                            );
                        }
                        if text.len() > 500 {
                            println!(
                                "{}   {}",
                                ui::theme::paint(&t, "accent", "\u{258C}", true),
                                ui::theme::paint_dim(&t, "... (truncated)")
                            );
                        }
                    }
                    Err(e) => {
                        println!(
                            "{}   {}",
                            ui::theme::paint_error_label(&t, "\u{2717}"),
                            ui::theme::paint(&t, "error", &e.to_string(), false)
                        );
                    }
                }
                println!("{}", ui::theme::paint_rail_empty(&t));
            }
            Err(e) => {
                println!(
                    "{} {}",
                    ui::theme::paint_error_label(&t, "\u{2717}"),
                    ui::theme::paint(&t, "error", &format!("task failed: {e}"), false)
                );
            }
        }
    }

    println!("{}", ui::theme::paint_rail_empty(&t));
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
