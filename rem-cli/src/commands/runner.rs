//! CLI subcommand runner functions.
//! Each function implements a top-level CLI subcommand
//! (ask, pull, explain, patch, new, theme, index, pipe).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use walkdir::WalkDir;

use crate::cli::{AppConfig, AskArgs, ExplainArgs, IndexArgs, NewArgs, PatchArgs, PullArgs, ThemeArgs};
use crate::config;
use crate::constants::CHAT_SYSTEM_PROMPT_CONVERSATIONAL;
use crate::find;
use crate::indexer;
use crate::intent::{classify_intent, TaskIntent};
use crate::provider::Provider;
use crate::templates;
use crate::text_util::truncate_bytes;
use crate::types::{file_icon, ModelReply};
use crate::ui::output::{print_banner, print_reply, SpinnerGuard};
use crate::ui::theme;

pub(crate) async fn run_pipe(client: &Provider, _cfg: &AppConfig, input: &str, verbose: bool) -> Result<()> {
    let t = theme::active();
    let prompt = if input.len() > 12000 {
        let truncated: String = input.chars().take(12000).collect();
        format!(
            "Analyze the following piped input. Be concise.\n\n{}...\n[truncated]",
            truncated
        )
    } else {
        format!("Analyze the following piped input. Be concise.\n\n{}", input)
    };
    let _spinner = SpinnerGuard::new("thinking...");
    let result = client.complete_chat_stream(
        &prompt,
        "[MODE: CHAT] You are in pipe/non-interactive mode. Respond concisely. No code generation unless explicitly asked.",
        "",
    ).await;
    match result {
        Ok(text) => {
            if verbose {
                eprintln!("\n  {} raw:\n{}\n", theme::paint_dim(&t, "verbose:"), text);
            }
            // Show a brief header with provider/model context
            let rail = theme::paint(&t, "accent", "\u{258C}", true);
            let model_tag = theme::paint(&t, "accent", &client.provider_label(), false);
            let hint = theme::paint_dim(&t, "(piped input)");
            println!();
            println!("{rail} {model_tag} {hint}");
            println!("{rail} {}", text.trim());
            println!();
            Ok(())
        }
        Err(e) => Err(e),
    }
}

pub(crate) async fn run_ask(client: &Provider, cfg: &AppConfig, args: AskArgs, verbose: bool) -> Result<()> {
    let mut composed = args.prompt;
    if let Some(path) = args.file {
        let ctx = build_context(&path, cfg.max_context_bytes, None)?;
        composed = format!("{}\n\nFile context:\n{}", composed, ctx);
    }
    let t = theme::active();
    print_banner(client);

    let intent = classify_intent(&composed);

    let _spinner = SpinnerGuard::new("thinking...");
    let result = if intent == TaskIntent::CodeAction {
        client.complete_json(&composed).await
    } else {
        let system_prompt = CHAT_SYSTEM_PROMPT_CONVERSATIONAL;
        let text = client.complete_chat_stream(&composed, system_prompt, "").await?;
        Ok(ModelReply {
            explanation: text.trim().to_string(),
            code: String::new(),
            files: vec![],
            commands: vec![],
            checks: vec![],
            caution: String::new(),
        })
    };

    let reply = result?;
    match args.format.as_str() {
        "json" => {
            let json = serde_json::to_string(&reply)
                .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {}\"}}", e));
            println!("{}", json);
        }
        "json-pretty" => {
            let json = serde_json::to_string_pretty(&reply)
                .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {}\"}}", e));
            println!("{}", json);
        }
        _ => {
            if verbose {
                eprintln!(
                    "{} raw explanation: {}",
                    theme::paint_dim(&t, "verbose:"),
                    reply.explanation
                );
                eprintln!("{} raw files: {:?}", theme::paint_dim(&t, "verbose:"), reply.files);
            }
            print_reply(&reply, true);
        }
    }
    Ok(())
}

pub(crate) fn run_pull(args: PullArgs, cfg: &AppConfig) -> Result<()> {
    let model = args.model.unwrap_or_else(|| cfg.model.clone());
    let t = theme::active();
    println!(
        "{} pulling model '{}' via Ollama...",
        theme::paint(&t, "accent", "\u{258C}", true),
        model
    );
    let status = std::process::Command::new("ollama")
        .args(["pull", &model])
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .context("failed to run ollama pull. Is Ollama installed?")?;
    if status.success() {
        println!(
            "{} model '{}' pulled successfully",
            theme::paint_success_label(&t, "\u{2713}"),
            model
        );
        Ok(())
    } else {
        Err(anyhow!("failed to pull model '{}'", model))
    }
}

pub(crate) async fn run_explain(client: &Provider, args: ExplainArgs) -> Result<()> {
    print_banner(client);
    let prompt = format!(
        "Explain this terminal command for a beginner and suggest a safer variant if needed: {}",
        args.command
    );

    let _spinner = SpinnerGuard::new("thinking...");
    let reply = client.complete_json(&prompt).await?;
    print_reply(&reply, false);
    Ok(())
}

pub(crate) async fn run_patch(client: &Provider, cfg: &AppConfig, args: PatchArgs) -> Result<()> {
    let t = theme::active();
    print_banner(client);
    let existing = fs::read_to_string(&args.file).with_context(|| format!("failed to read {}", args.file.display()))?;
    let dir_ctx = build_context(&args.file, cfg.max_context_bytes, Some(&existing))?;
    let prompt = format!(
        "Task: {}\n\nTarget file: {}\n\nCurrent content:\n{}\n\nNearby context:\n{}\n\nReturn updated file content in code or files array.",
        args.task, args.file.display(), existing, dir_ctx
    );

    let _spinner = SpinnerGuard::new("thinking...");
    let reply = client.complete_json(&prompt).await?;
    println!(
        "{}",
        theme::paint(
            &t,
            "accent",
            &format!("Patch preview for {}", args.file.display()),
            true
        )
    );
    print_reply(&reply, true);
    Ok(())
}

fn build_context(target: &Path, max_bytes: usize, existing_content: Option<&str>) -> Result<String> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let mut out = String::from("Directory snapshot:\n");
    for entry in WalkDir::new(parent).max_depth(2) {
        let entry = entry?;
        if entry.depth() == 0 {
            continue;
        }
        if entry.file_type().is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                if find::should_skip_dir(name) {
                    continue;
                }
            }
        }
        if entry.file_type().is_file() {
            if let Some(name) = entry.file_name().to_str() {
                if find::should_skip_file(name) {
                    continue;
                }
            }
        }
        let p = entry.path();
        let rel = p.strip_prefix(parent).unwrap_or(p);
        out.push_str(&format!("- {}\n", rel.display()));
        if out.len() > max_bytes {
            break;
        }
    }
    if target.exists() || existing_content.is_some() {
        let content = match existing_content {
            Some(c) => c.to_string(),
            None => fs::read_to_string(target).with_context(|| format!("failed to read {}", target.display()))?,
        };
        out.push_str("\nTarget file:\n");
        out.push_str(&truncate_bytes(&content, max_bytes / 2));
    }
    Ok(truncate_bytes(&out, max_bytes))
}

pub(crate) fn run_new(args: NewArgs, cfg: &AppConfig) -> Result<()> {
    let t = theme::active();
    let dir = if args.name.starts_with('/') || args.name.starts_with("./") || args.name.starts_with("../") {
        PathBuf::from(&args.name)
    } else if let Some(ref ws) = cfg.workspace_dir {
        let base = PathBuf::from(ws);
        base.join(&args.name)
    } else {
        PathBuf::from(&args.name)
    };

    if dir.exists() {
        return Err(anyhow!(
            "Directory '{}' already exists. Choose a different name.",
            dir.display()
        ));
    }

    let files = match args.project_type.as_str() {
        "bare" => templates::template_bare(&args.name),
        "portfolio" => templates::template_portfolio(&args.name),
        "landing" => templates::template_landing(&args.name),
        "blog" => templates::template_blog(&args.name),
        "rust" => templates::template_rust(&args.name),
        "python" => templates::template_python(&args.name),
        "go" => templates::template_go(&args.name),
        "javascript" => templates::template_javascript(&args.name),
        other => {
            return Err(anyhow!(
                "Unknown project type '{}'. Choose: bare, portfolio, landing, blog, rust, python, go, javascript",
                other
            ))
        }
    };

    for file in &files {
        let file_path = dir.join(&file.path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file_path, &file.content)?;
    }

    println!(
        "{} {}",
        theme::paint_success_label(&t, "\u{2713}"),
        theme::paint_bright(&t, &format!("created project '{}' ({})", args.name, args.project_type))
    );
    for f in &files {
        let icon = file_icon(&f.path);
        println!(
            "  {} {} ({} bytes)",
            icon,
            theme::paint_bright(&t, &f.path),
            f.content.len()
        );
    }
    println!();
    println!("{} cd {} && open index.html", theme::paint_dim(&t, "next:"), args.name);

    Ok(())
}

pub(crate) fn run_theme(args: ThemeArgs) -> Result<()> {
    let t = theme::active();
    if let Some(name) = &args.name {
        let upper = name.to_uppercase();
        let names = theme::list_names();
        if names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            theme::set_active(&upper);
            let mut cfg = config::load_config().unwrap_or_default();
            cfg.theme = upper.clone();
            let _ = config::save_config(&cfg);
            println!(
                "{} theme switched to {}",
                theme::paint_success_label(&t, "\u{2713}"),
                theme::paint_bright(&t, &upper)
            );
        } else {
            println!(
                "{} unknown theme '{}'. Available: {}",
                theme::paint_warning(&t, "!"),
                name,
                names.join(", ")
            );
        }
    } else {
        println!("{}", theme::paint_rail_header(&t, "THEMES"));
        for name in theme::list_names() {
            println!("{}   {}", theme::paint(&t, "accent", "\u{258C}", true), name);
        }
        println!("{}", theme::paint_rail_empty(&t));
        println!(
            "{} use {} {}",
            theme::paint(&t, "accent", "\u{258C}", true),
            theme::paint_bright(&t, "rem theme <name>"),
            theme::paint_dim(&t, "or /theme <name> in chat to switch")
        );
        println!("{}", theme::paint(&t, "accent", "\u{258C}", true));
    }
    Ok(())
}

pub(crate) async fn run_index(args: IndexArgs, cfg: &AppConfig) -> Result<()> {
    let t = theme::active();
    let dir = args.dir.clone().unwrap_or_else(|| {
        cfg.workspace_dir
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    });
    let dir = if dir.exists() { dir } else { PathBuf::from(".") };

    println!("{}", theme::paint(&t, "accent", "\u{258C}", true));
    println!(
        "{} {} {}",
        theme::paint(&t, "accent", "\u{258C}", true),
        theme::paint_bright(&t, "rem index"),
        theme::paint_dim(&t, "\u{2014} codebase retrieval index (pure Rust)")
    );
    println!(
        "{} target: {}",
        theme::paint(&t, "accent", "\u{258C}", true),
        theme::paint_dim(&t, &dir.display().to_string())
    );

    let refreshing = indexer::load_codebase_index(&dir).is_some();
    let (mut chunks, file_mtimes) = indexer::generate_codebase_index(&dir)?;
    if chunks.is_empty() {
        println!(
            "{} {} no indexable files found (after skips)",
            theme::paint(&t, "accent", "\u{258C}", true),
            theme::paint_warning(&t, "\u{26A0}")
        );
        return Ok(());
    }

    if args.embeddings {
        println!(
            "{} {} computing embeddings (nomic-embed-text via Ollama)...",
            theme::paint(&t, "accent", "\u{258C}", true),
            theme::paint_dim(&t, "\u{2699}")
        );
        indexer::compute_embeddings(&mut chunks, &cfg.ollama_url).await;
        let embedded = chunks.iter().filter(|c| c.embedding.is_some()).count();
        println!(
            "{} {} {} chunks embedded",
            theme::paint(&t, "accent", "\u{258C}", true),
            theme::paint_success_label(&t, "\u{2713}"),
            embedded
        );
    }

    if args.dry_run {
        println!(
            "{} {} {} dry-run: would index {} chunk(s) across {} file(s)",
            theme::paint(&t, "accent", "\u{258C}", true),
            theme::paint(&t, "accent", "\u{258C}", true),
            theme::paint_dim(&t, "DRY-RUN"),
            chunks.len(),
            chunks.iter().map(|c| &c.path).collect::<HashSet<_>>().len(),
        );
        return Ok(());
    }

    let num_chunks = chunks.len();
    let unique_files = chunks.iter().map(|c| &c.path).collect::<HashSet<_>>().len();
    indexer::write_codebase_index(&dir, chunks, file_mtimes)?;

    let out_path = dir.join(".rem/codebase_index.json");
    let action = if refreshing { "refreshed" } else { "created" };
    println!(
        "{} {} {} {} chunks from {} files",
        theme::paint(&t, "accent", "\u{258C}", true),
        theme::paint_success_label(&t, "\u{2713}"),
        action,
        num_chunks,
        unique_files
    );
    println!(
        "{} index: {}",
        theme::paint(&t, "accent", "\u{258C}", true),
        theme::paint_bright(&t, &out_path.display().to_string())
    );
    println!(
        "{} `rem chat` / `rem ask` / `/goal` will now pull relevant chunks instead of full listings.",
        theme::paint(&t, "accent", "\u{258C}", true)
    );
    println!(
        "{} (keyword retrieval; raise model_ctx in ~/.config/rem-cli/config.toml for large projects)",
        theme::paint(&t, "accent", "\u{258C}", true)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{AppConfig, NewArgs, ThemeArgs};

    /// Guard that removes the directory on drop (cleanup on panic too).
    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn unique_dir(prefix: &str) -> (std::path::PathBuf, TempDir) {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("rem-unit-{}-{}-{}", prefix, std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        let guard = TempDir(dir.clone());
        (dir, guard)
    }

    // -----------------------------------------------------------------------
    // build_context tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_context_empty_dir() {
        let (dir, _guard) = unique_dir("build-empty");
        let file = dir.join("empty.txt");
        std::fs::write(&file, "").unwrap();
        let result = build_context(&file, 16_000, None).unwrap();
        assert!(result.contains("Directory snapshot"), "result: {result}");
        assert!(result.contains("empty.txt"), "result: {result}");
    }

    #[test]
    fn test_build_context_with_file() {
        let (dir, _guard) = unique_dir("build-file");
        let file = dir.join("hello.txt");
        std::fs::write(&file, "Hello, world!").unwrap();
        let result = build_context(&file, 16_000, None).unwrap();
        assert!(result.contains("Directory snapshot"));
        assert!(result.contains("hello.txt"));
        assert!(result.contains("Hello, world!"));
    }

    #[test]
    fn test_build_context_respects_max_bytes() {
        let (dir, _guard) = unique_dir("build-trunc");
        let file = dir.join("big.txt");
        let big = "A".repeat(10_000);
        std::fs::write(&file, &big).unwrap();
        let result = build_context(&file, 200, None).unwrap();
        assert!(
            result.len() < 500,
            "expected output to be truncated, got {} bytes: {result}",
            result.len()
        );
        assert!(result.contains("Directory snapshot"));
        assert!(result.contains("truncated"));
    }

    #[test]
    fn test_build_context_with_existing_content() {
        let (dir, _guard) = unique_dir("build-preexisting");
        let file = dir.join("hello.txt");
        std::fs::write(&file, "Goodbye, world!").unwrap();
        // Pass pre-read content that differs from file on disk
        let result = build_context(&file, 16_000, Some("Hello, world!")).unwrap();
        assert!(result.contains("Hello, world!"));
        assert!(!result.contains("Goodbye, world!"));
    }

    // -----------------------------------------------------------------------
    // run_new tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_run_new_bare() {
        let (dir, _guard) = unique_dir("new-bare");
        let cfg = AppConfig {
            workspace_dir: Some(dir.to_string_lossy().to_string()),
            ..AppConfig::default()
        };
        let args = NewArgs {
            name: "test-bare".into(),
            project_type: "bare".into(),
        };
        run_new(args, &cfg).unwrap();
        let project = dir.join("test-bare");
        assert!(project.join("index.html").exists(), "index.html missing");
        assert!(project.join("style.css").exists(), "style.css missing");
        assert!(project.join("script.js").exists(), "script.js missing");
    }

    #[test]
    fn test_run_new_rust() {
        let (dir, _guard) = unique_dir("new-rust");
        let cfg = AppConfig {
            workspace_dir: Some(dir.to_string_lossy().to_string()),
            ..AppConfig::default()
        };
        let args = NewArgs {
            name: "test-rust".into(),
            project_type: "rust".into(),
        };
        run_new(args, &cfg).unwrap();
        let project = dir.join("test-rust");
        assert!(project.join("Cargo.toml").exists(), "Cargo.toml missing");
        assert!(project.join("src/main.rs").exists(), "src/main.rs missing");
    }

    #[test]
    fn test_run_new_unknown_type() {
        let (dir, _guard) = unique_dir("new-unknown");
        let cfg = AppConfig {
            workspace_dir: Some(dir.to_string_lossy().to_string()),
            ..AppConfig::default()
        };
        let args = NewArgs {
            name: "test-unknown".into(),
            project_type: "nonexistent99".into(),
        };
        let err = run_new(args, &cfg).unwrap_err();
        assert!(err.to_string().contains("Unknown project type"), "err: {err}");
    }

    // -----------------------------------------------------------------------
    // run_theme tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_run_theme_list() {
        let args = ThemeArgs { name: None };
        assert!(run_theme(args).is_ok());
    }

    #[test]
    fn test_run_theme_switch() {
        let args = ThemeArgs {
            name: Some("GHOST".into()),
        };
        assert!(run_theme(args).is_ok());
    }
}
