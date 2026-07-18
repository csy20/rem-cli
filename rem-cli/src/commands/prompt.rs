use std::path::PathBuf;
use std::sync::LazyLock;

use crate::chat::ChatSession;
use crate::ui;

/// Pre-compiled regex for matching prompt template variables like {{name}}.
static TEMPLATE_VAR_RE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\{\{(\w+)\}\}").unwrap());

fn prompts_dir(session: &ChatSession) -> Option<PathBuf> {
    let dir = session.ctx.project_dir.as_ref()?;
    let prompts = dir.join(".rem").join("prompts");
    Some(prompts)
}

/// Save the current input as a prompt template (`/prompt save <name>`).
pub(crate) fn handle_prompt_save(session: &ChatSession, name: &str) {
    let t = ui::theme::active();
    let name = name.trim();
    if name.is_empty() {
        println!(
            "{} usage: /prompt save <name>",
            ui::theme::paint_warning(&t, "\u{258C}")
        );
        return;
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        println!(
            "{} name must be alphanumeric (underscores/dashes allowed)",
            ui::theme::paint_warning(&t, "\u{258C}")
        );
        return;
    }
    let last_input = session.last_user_input.trim();
    if last_input.is_empty() {
        println!(
            "{} no input to save — send a message first",
            ui::theme::paint_warning(&t, "\u{258C}")
        );
        return;
    }
    let dir = match prompts_dir(session) {
        Some(d) => d,
        None => {
            println!("{} no project directory set", ui::theme::paint_warning(&t, "\u{258C}"));
            return;
        }
    };
    if std::fs::create_dir_all(&dir).is_err() {
        println!(
            "{} failed to create prompts directory",
            ui::theme::paint_error_label(&t, "\u{2717}")
        );
        return;
    }
    let path = dir.join(format!("{}.md", name));
    if path.exists() {
        println!(
            "{} template '{}' already exists — overwrite with /prompt save {}! (with !)",
            ui::theme::paint_warning(&t, "\u{258C}"),
            name,
            name
        );
        return;
    }
    match std::fs::write(&path, last_input) {
        Ok(()) => println!(
            "{} saved prompt template '{}'",
            ui::theme::paint_success_label(&t, "\u{2713}"),
            name
        ),
        Err(e) => println!(
            "{} failed to save template: {}",
            ui::theme::paint_error_label(&t, "\u{2717}"),
            e
        ),
    }
}

/// Load and return a prompt template (`/prompt load <name>`).
pub(crate) fn handle_prompt_load(session: &ChatSession, name: &str) -> Option<String> {
    let t = ui::theme::active();
    let name = name.trim();
    if name.is_empty() {
        println!(
            "{} usage: /prompt load <name>",
            ui::theme::paint_warning(&t, "\u{258C}")
        );
        return None;
    }
    let dir = match prompts_dir(session) {
        Some(d) => d,
        None => {
            println!("{} no project directory set", ui::theme::paint_warning(&t, "\u{258C}"));
            return None;
        }
    };
    let path = dir.join(format!("{}.md", name));
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c.trim().to_string(),
        Err(_) => {
            println!(
                "{} template '{}' not found",
                ui::theme::paint_warning(&t, "\u{258C}"),
                name
            );
            return None;
        }
    };
    // Substitute {{variable}} placeholders using pre-compiled regex
    let mut result = content.clone();
    for cap in TEMPLATE_VAR_RE.captures_iter(&content) {
        let var_name = cap.get(1).unwrap().as_str();
        print!("{} value for '{}': ", ui::theme::paint_bright(&t, "?"), var_name);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() {
            let val = input.trim();
            result = result.replace(&format!("{{{{{}}}}}", var_name), val);
        }
    }
    Some(result)
}

/// List all saved prompt templates (`/prompt list`).
pub(crate) fn handle_prompt_list(session: &ChatSession) {
    let t = ui::theme::active();
    let dir = match prompts_dir(session) {
        Some(d) => d,
        None => {
            println!("{} no project directory set", ui::theme::paint_warning(&t, "\u{258C}"));
            return;
        }
    };
    if !dir.exists() {
        println!("{} no prompt templates saved", ui::theme::paint_dim(&t, "\u{258C}"));
        return;
    }
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = entry.path().file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort();
    if names.is_empty() {
        println!("{} no prompt templates saved", ui::theme::paint_dim(&t, "\u{258C}"));
        return;
    }
    let rail = ui::theme::paint_rail_empty(&t);
    println!("{rail}");
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, "saved prompt templates")
    );
    for name in &names {
        println!(
            "{}   {} {}",
            rail,
            ui::theme::paint_dim(&t, "\u{2022}"),
            ui::theme::paint(&t, "accent", name, false)
        );
    }
    println!("{rail}");
}

/// Delete a saved prompt template (`/prompt delete <name>`).
pub(crate) fn handle_prompt_delete(session: &ChatSession, name: &str) {
    let t = ui::theme::active();
    let name = name.trim();
    if name.is_empty() {
        println!(
            "{} usage: /prompt delete <name>",
            ui::theme::paint_warning(&t, "\u{258C}")
        );
        return;
    }
    let dir = match prompts_dir(session) {
        Some(d) => d,
        None => {
            println!("{} no project directory set", ui::theme::paint_warning(&t, "\u{258C}"));
            return;
        }
    };
    let path = dir.join(format!("{}.md", name));
    match std::fs::remove_file(&path) {
        Ok(()) => println!(
            "{} deleted template '{}'",
            ui::theme::paint_success_label(&t, "\u{2713}"),
            name
        ),
        Err(_) => println!(
            "{} template '{}' not found",
            ui::theme::paint_warning(&t, "\u{258C}"),
            name
        ),
    }
}

/// Overwrite a saved prompt template (`/prompt save <name>!`).
pub(crate) fn handle_prompt_save_force(session: &ChatSession, name: &str) {
    let t = ui::theme::active();
    let name = name.trim_end_matches('!').trim();
    if name.is_empty() {
        return;
    }
    let last_input = session.last_user_input.trim();
    if last_input.is_empty() {
        println!("{} no input to save", ui::theme::paint_warning(&t, "\u{258C}"));
        return;
    }
    let dir = match prompts_dir(session) {
        Some(d) => d,
        None => {
            println!("{} no project directory set", ui::theme::paint_warning(&t, "\u{258C}"));
            return;
        }
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join(format!("{}.md", name));
    match std::fs::write(&path, last_input) {
        Ok(()) => println!(
            "{} saved prompt template '{}'",
            ui::theme::paint_success_label(&t, "\u{2713}"),
            name
        ),
        Err(e) => println!(
            "{} failed to save template: {}",
            ui::theme::paint_error_label(&t, "\u{2717}"),
            e
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ChatSession;

    fn make_session(tmp: &std::path::Path) -> ChatSession {
        let mut s = ChatSession::new("test", Some(tmp.to_path_buf())).unwrap();
        // Simulate a user input so save works
        s.last_user_input = "write a fibonacci function in rust".to_string();
        s
    }

    #[test]
    fn prompts_dir_returns_none_without_project() {
        let session = ChatSession::new("test", None).unwrap();
        assert!(prompts_dir(&session).is_none());
    }

    #[test]
    fn handle_prompt_save_and_list() {
        let tmp = std::env::temp_dir().join(format!("rem-test-prompt-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let session = make_session(&tmp);

        handle_prompt_save(&session, "test-helper");
        // List should show it
        handle_prompt_list(&session);

        // Verify file exists
        let prompts = tmp.join(".rem/prompts/test-helper.md");
        assert!(prompts.exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_prompt_save_empty_name_shows_warning() {
        let tmp = std::env::temp_dir().join(format!("rem-test-prompt-e-{}", std::process::id()));
        let s = ChatSession::new("test", Some(tmp.clone())).unwrap();
        handle_prompt_save(&s, "");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_prompt_load_nonexistent() {
        let tmp = std::env::temp_dir().join(format!("rem-test-prompt-l-{}", std::process::id()));
        let s = ChatSession::new("test", Some(tmp.clone())).unwrap();
        let result = handle_prompt_load(&s, "nonexistent");
        assert!(result.is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_prompt_list_empty() {
        let tmp = std::env::temp_dir().join(format!("rem-test-prompt-le-{}", std::process::id()));
        let s = ChatSession::new("test", Some(tmp.clone())).unwrap();
        handle_prompt_list(&s);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_prompt_delete_nonexistent() {
        let tmp = std::env::temp_dir().join(format!("rem-test-prompt-d-{}", std::process::id()));
        let s = ChatSession::new("test", Some(tmp.clone())).unwrap();
        handle_prompt_delete(&s, "nonexistent");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_prompt_save_force_overwrites() {
        let tmp = std::env::temp_dir().join(format!("rem-test-prompt-f-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let session = make_session(&tmp);

        handle_prompt_save(&session, "testme");
        handle_prompt_save_force(&session, "testme!");

        let prompts = tmp.join(".rem/prompts/testme.md");
        assert!(prompts.exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
