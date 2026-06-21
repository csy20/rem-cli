//! Chat session I/O, context building, and prompt rendering.
//! Free functions that operate on/around [`ChatSession`] for system diagnostics,
//! project context, welcome banners, prompt strings, and response validation.

use std::path::Path;

use ignore::WalkBuilder;
use walkdir::WalkDir;

use crate::chat::{ChatSession, RunMode};
use crate::intent::TaskIntent;
use crate::parsing::strip_code_blocks;
use crate::provider::{Provider, ProviderKind};
use crate::ui;

/// Warns if system RAM is 16 GB or less (important for local LLM performance).
pub(crate) fn check_system_resources() {
    let t = ui::theme::active();
    let mem_gb = detect_system_ram_gb();
    if mem_gb > 0 && mem_gb <= 16 {
        eprintln!(
            "{} {} GB RAM detected \u{2014} Ollama may be slow on CPU.",
            ui::theme::paint_warning(&t, "\u{258C} system:"),
            mem_gb
        );
        eprintln!(
            "{} Try:  OLLAMA_NUM_PARALLEL=1 OLLAMA_MAX_LOADED_MODELS=1 ollama serve",
            ui::theme::paint_rail_empty(&t)
        );
        eprintln!();
    }
}

fn detect_system_ram_gb() -> u64 {
    // Linux: /proc/meminfo
    if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if line.starts_with("MemTotal:") {
                let kb: u64 = line.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                return kb / 1024 / 1024;
            }
        }
    }

    // macOS: sysctl hw.memsize
    if let Ok(output) = std::process::Command::new("sysctl").args(["-n", "hw.memsize"]).output() {
        if let Ok(s) = String::from_utf8(output.stdout) {
            if let Ok(bytes) = s.trim().parse::<u64>() {
                return bytes / 1024 / 1024 / 1024;
            }
        }
    }

    0
}

/// Prints the welcome banner with model and mode information.
pub(crate) fn print_welcome(client: &Provider) {
    println!();
    ui::header::render(&client.provider_label(), "CHAT");
    println!();
}

/// Builds a file tree listing of the project directory (depth-limited, size-capped).
pub(crate) fn build_project_context(dir: &Path, max_bytes: usize) -> String {
    let mut out = String::from("Project files:\n");
    let mut count = 0u32;
    let max_depth = 4;

    let mut entries: Vec<String> = Vec::new();
    for entry in WalkBuilder::new(dir)
        .max_depth(Some(max_depth as usize))
        .sort_by_file_name(|a, b| a.cmp(b))
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build()
        .flatten()
    {
        let p = entry.path();
        let Ok(rel) = p.strip_prefix(dir) else {
            continue;
        };
        let rel_str = rel.display().to_string();
        if rel_str.is_empty() {
            continue;
        }
        if rel_str.starts_with('.') && rel_str != "." {
            continue;
        }
        if rel_str.contains("venv")
            || rel_str.contains("dist")
            || rel_str.contains(".pytest_cache")
            || rel
                .components()
                .any(|c| c.as_os_str().to_str().is_some_and(crate::find::should_skip_dir))
        {
            continue;
        }

        if p.is_dir() {
            if rel.components().count() >= 3 {
                continue;
            }
            entries.push(format!("{}/", rel_str));
        } else {
            let size = p.metadata().map(|m| m.len()).unwrap_or(0);
            entries.push(format!("{}  ({} bytes)", rel_str, size));
        }
        count += 1;
        if out.len() > max_bytes {
            break;
        }
    }

    if count > 0 {
        out.push_str(&entries.join("\n"));
        out.push_str("\n\n");
        out
    } else {
        String::new()
    }
}

/// Detects project language type from files in directory (Cargo.toml \u{2192} rust, etc.).
pub(crate) fn detect_project_type(dir: &Path) -> &'static str {
    if !dir.exists() {
        return "";
    }
    let entries: Vec<String> = WalkDir::new(dir)
        .max_depth(1)
        .into_iter()
        .filter_map(|e: Result<walkdir::DirEntry, walkdir::Error>| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.file_name().to_string_lossy().to_lowercase())
        .collect();

    let has_file = |name: &str| entries.iter().any(|f| f == name);

    if has_file("cargo.toml") {
        return "rust";
    }
    if has_file("go.mod") {
        return "go";
    }
    if has_file("pyproject.toml") || has_file("setup.py") || has_file("requirements.txt") {
        return "python";
    }
    if has_file("package.json") {
        return "javascript";
    }
    if has_file("index.html") && has_file("style.css") {
        return "html_css";
    }
    if has_file("dart.yaml") || has_file("pubspec.yaml") {
        return "dart";
    }
    if has_file("makefile") {
        return "cpp";
    }
    ""
}

/// Returns language-specific guidance text for the system prompt.
pub(crate) fn language_specific_guidance(project_type: &str) -> &'static str {
    match project_type {
        "rust" => "\nLanguage context: Rust project. Use cargo build/run. Prefer &str over String where possible. Include Cargo.toml deps.",
        "go" => "\nLanguage context: Go project. Use go mod tidy. Follow standard library patterns.",
        "python" => "\nLanguage context: Python project. Use pip install for deps. Follow PEP 8. Use type hints.",
        "javascript" => "\nLanguage context: JavaScript/Node.js project. Use npm/yarn. Prefer ES modules. Include package.json deps.",
        "html_css" => "\nLanguage context: HTML/CSS project. Use semantic HTML. Responsive CSS with modern layout (flexbox/grid).",
        "dart" => "\nLanguage context: Dart/Flutter project. Use pub get for deps. Follow effective Dart guidelines.",
        "cpp" => "\nLanguage context: C/C++ project. Use make/gcc. Show compilation commands.",
        _ => "",
    }
}

/// Builds the styled terminal prompt string (e.g., `[CODE] ollama/rem-coder>`).
pub(crate) fn build_prompt(session: &ChatSession, client: &Provider) -> String {
    let t = ui::theme::active();
    let model_short = client.model.split(':').next().unwrap_or(&client.model);
    let mode_key = ui::theme::accent_for_mode(session.mode.label());
    let provider_prefix = match client.kind {
        ProviderKind::Ollama => "",
        _ => client.kind.as_str(),
    };
    let mut p = String::new();
    p.push('\x01');
    p.push_str(t.fg(mode_key));
    p.push('\x02');
    p.push('[');
    p.push_str(session.mode.label());
    p.push(']');
    p.push_str("\x01\x1b[0m\x02");
    p.push(' ');
    p.push('\x01');
    p.push_str(t.fg("accent"));
    p.push('\x02');
    if !provider_prefix.is_empty() {
        p.push_str(provider_prefix);
        p.push('/');
    }
    p.push_str(model_short);
    p.push('>');
    p.push_str("\x01\x1b[0m\x02");
    p.push(' ');
    p
}

/// Validates the LLM response, stripping code if in CHAT mode and inappropriate.
/// Returns (was_validated, cleaned_response).
pub(crate) fn validate_chat_response(response: &str, intent: &TaskIntent, mode: &RunMode) -> (bool, String) {
    if *intent != TaskIntent::CodeAction && *mode != RunMode::Code {
        let has_code_fences = response.contains("```");
        let has_multi_file = response.contains("### ") && has_code_fences;
        let has_json =
            response.trim().starts_with('{') && (response.contains("\"code\"") || response.contains("\"files\""));

        if has_multi_file || has_json {
            let code_stripped = strip_code_blocks(response);
            if !code_stripped.trim().is_empty() {
                return (true, code_stripped);
            }
            return (true, "I understood your question. Let me answer directly: ".to_string());
        }
    }

    if response.trim().is_empty() {
        return (
            true,
            "(No response generated \u{2014} please try again or rephrase)".to_string(),
        );
    }

    (false, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::RunMode;
    use crate::intent::TaskIntent;
    use std::path::PathBuf;

    #[test]
    fn validate_chat_response_passes_plain_text() {
        let (was_validated, _) = validate_chat_response("Hi there!", &TaskIntent::FastAnswer, &RunMode::Chat);
        assert!(!was_validated);
    }

    #[test]
    fn validate_chat_allows_code_in_code_mode() {
        let (was_validated, _) = validate_chat_response(
            "### app.js\n```js\nconst x = 1;\n```",
            &TaskIntent::CodeAction,
            &RunMode::Code,
        );
        assert!(!was_validated);
    }

    #[test]
    fn validate_chat_strips_code_in_chat_mode() {
        let (was_validated, cleaned) = validate_chat_response(
            "Here's the code:\n### app.js\n```js\nconst x = 1;\n```\n done",
            &TaskIntent::FastAnswer,
            &RunMode::Chat,
        );
        assert!(was_validated);
        assert!(!cleaned.contains("```"));
    }

    #[test]
    fn validate_chat_strips_json_in_chat_mode() {
        let (was_validated, _cleaned) =
            validate_chat_response("{\"code\": \"fn main() {}\"}", &TaskIntent::FastAnswer, &RunMode::Chat);
        assert!(was_validated);
    }

    #[test]
    fn validate_chat_handles_empty_response() {
        let (was_validated, cleaned) = validate_chat_response("", &TaskIntent::CodeAction, &RunMode::Chat);
        assert!(was_validated);
        assert!(cleaned.contains("No response generated"));
    }

    #[test]
    fn detect_project_type_rust() {
        let dir = std::env::temp_dir().join(format!("rem-test-rust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        assert_eq!(detect_project_type(&dir), "rust");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_project_type_python() {
        let dir = std::env::temp_dir().join(format!("rem-test-py-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("setup.py"), "").unwrap();
        assert_eq!(detect_project_type(&dir), "python");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_project_type_unknown() {
        let dir = std::env::temp_dir().join(format!("rem-test-unk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("readme.md"), "").unwrap();
        assert_eq!(detect_project_type(&dir), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_project_type_nonexistent() {
        let dir = PathBuf::from("/nonexistent_rem_test_dir");
        assert_eq!(detect_project_type(&dir), "");
    }

    #[test]
    fn language_specific_guidance_returns_known_types() {
        assert!(language_specific_guidance("rust").contains("cargo"));
        assert!(language_specific_guidance("python").contains("PEP 8"));
        assert!(language_specific_guidance("javascript").contains("npm"));
        assert!(language_specific_guidance("go").contains("go mod"));
        assert!(language_specific_guidance("html_css").contains("flexbox"));
    }

    #[test]
    fn language_specific_guidance_unknown_returns_empty() {
        assert_eq!(language_specific_guidance("unknown_type"), "");
    }
}
