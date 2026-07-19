use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Path to the compiled `rem` binary.
fn rem_binary() -> &'static str {
    env!("CARGO_BIN_EXE_rem")
}

fn unique_dir(prefix: &str) -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("rem-int-{prefix}-{pid}-{ts}-{n}"))
}

fn create_temp_dir(prefix: &str) -> (std::path::PathBuf, TempDir) {
    let dir = unique_dir(prefix);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let guard = TempDir(dir.clone());
    (dir, guard)
}

struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn help_flag_shows_usage() {
    let output = Command::new(rem_binary())
        .arg("--help")
        .output()
        .expect("failed to run rem --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"), "stdout: {stdout}");
    assert!(stdout.contains("ask"), "stdout: {stdout}");
    assert!(stdout.contains("chat"), "stdout: {stdout}");
    assert!(stdout.contains("new"), "stdout: {stdout}");
    assert!(stdout.contains("explain"), "stdout: {stdout}");
}

#[test]
fn version_flag_prints_version() {
    let output = Command::new(rem_binary())
        .arg("--version")
        .output()
        .expect("failed to run rem --version");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty());
}

#[test]
fn new_command_scaffolds_bare_project() {
    let (root, _guard) = create_temp_dir("new-bare");
    let output = Command::new(rem_binary())
        .args(["new", "./test-bare", "--project-type", "bare"])
        .current_dir(&root)
        .output()
        .expect("failed to run rem new");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.join("test-bare/index.html").exists());
}

#[test]
fn new_command_scaffolds_portfolio_project() {
    let (root, _guard) = create_temp_dir("new-portfolio");
    let output = Command::new(rem_binary())
        .args(["new", "./test-portfolio", "--project-type", "portfolio"])
        .current_dir(&root)
        .output()
        .expect("failed to run rem new portfolio");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.join("test-portfolio/index.html").exists());
}

#[test]
fn new_command_scaffolds_landing_project() {
    let (root, _guard) = create_temp_dir("new-landing");
    let output = Command::new(rem_binary())
        .args(["new", "./test-landing", "--project-type", "landing"])
        .current_dir(&root)
        .output()
        .expect("failed to run rem new landing");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn new_command_scaffolds_blog_project() {
    let (root, _guard) = create_temp_dir("new-blog");
    let output = Command::new(rem_binary())
        .args(["new", "./test-blog", "--project-type", "blog"])
        .current_dir(&root)
        .output()
        .expect("failed to run rem new blog");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn new_command_scaffolds_rust_project() {
    let (root, _guard) = create_temp_dir("new-rust");
    let output = Command::new(rem_binary())
        .args(["new", "./test-rust", "--project-type", "rust"])
        .current_dir(&root)
        .output()
        .expect("failed to run rem new rust");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let project_dir = root.join("test-rust");
    assert!(project_dir.join("Cargo.toml").exists());
    assert!(project_dir.join("src/main.rs").exists());
}

#[test]
fn new_command_fails_on_existing_dir() {
    let (root, _guard) = create_temp_dir("new-existing");
    let _ = std::fs::create_dir_all(root.join("existing-project"));
    let output = Command::new(rem_binary())
        .args(["new", "./existing-project", "--project-type", "bare"])
        .current_dir(&root)
        .output()
        .expect("failed to run rem new on existing dir");
    assert!(!output.status.success(), "should fail when dir exists");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already exists"), "stderr: {stderr}");
}

#[test]
fn new_command_rejects_unknown_type() {
    let (root, _guard) = create_temp_dir("new-unknown");
    let output = Command::new(rem_binary())
        .args(["new", "./test-unknown", "--project-type", "nonexistent"])
        .current_dir(&root)
        .output()
        .expect("failed to run rem new with unknown type");
    assert!(!output.status.success(), "should fail for unknown type");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Unknown project type"), "stderr: {stderr}");
}

#[test]
fn index_command_handles_nonexistent_dir() {
    let root = unique_dir("index-nonexistent");
    let output = Command::new(rem_binary())
        .args(["index", root.to_str().unwrap()])
        .output()
        .expect("failed to run rem index on missing dir");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Falls back to cwd when dir doesn't exist
    assert!(output.status.success(), "stdout: {stdout}\nstderr: {stderr}");
}

#[test]
fn new_command_scaffolds_python_project() {
    let (root, _guard) = create_temp_dir("new-python");
    let output = Command::new(rem_binary())
        .args(["new", "./test-python", "--project-type", "python"])
        .current_dir(&root)
        .output()
        .expect("failed to run rem new python");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let project_dir = root.join("test-python");
    assert!(project_dir.join("main.py").exists());
    assert!(project_dir.join("requirements.txt").exists());
}

#[test]
fn new_command_scaffolds_go_project() {
    let (root, _guard) = create_temp_dir("new-go");
    let output = Command::new(rem_binary())
        .args(["new", "./test-go", "--project-type", "go"])
        .current_dir(&root)
        .output()
        .expect("failed to run rem new go");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let project_dir = root.join("test-go");
    assert!(project_dir.join("go.mod").exists());
    assert!(project_dir.join("main.go").exists());
}

#[test]
fn new_command_scaffolds_javascript_project() {
    let (root, _guard) = create_temp_dir("new-js");
    let output = Command::new(rem_binary())
        .args(["new", "./test-js", "--project-type", "javascript"])
        .current_dir(&root)
        .output()
        .expect("failed to run rem new javascript");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let project_dir = root.join("test-js");
    assert!(project_dir.join("package.json").exists());
    assert!(project_dir.join("index.js").exists());
}

#[test]
fn theme_command_lists_themes() {
    let output = Command::new(rem_binary())
        .arg("theme")
        .output()
        .expect("failed to run rem theme");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("GHOST"), "stdout: {stdout}");
    assert!(stdout.contains("PAPER"), "stdout: {stdout}");
    assert!(stdout.contains("SAKURA"), "stdout: {stdout}");
}

#[test]
fn theme_command_rejects_unknown_theme() {
    let output = Command::new(rem_binary())
        .args(["theme", "UNKNOWN_THEME_XYZ"])
        .output()
        .expect("failed to run rem theme UNKNOWN_THEME_XYZ");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("unknown theme") || stderr.contains("unknown theme"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
}

#[test]
fn new_command_help_shows_types() {
    let output = Command::new(rem_binary())
        .args(["new", "--help"])
        .output()
        .expect("failed to run rem new --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("bare"), "stdout: {stdout}");
    assert!(stdout.contains("portfolio"), "stdout: {stdout}");
    assert!(stdout.contains("blog"), "stdout: {stdout}");
    assert!(stdout.contains("rust"), "stdout: {stdout}");
    assert!(stdout.contains("python"), "stdout: {stdout}");
    assert!(stdout.contains("go"), "stdout: {stdout}");
    assert!(stdout.contains("javascript"), "stdout: {stdout}");
}

#[test]
fn index_command_dry_run_on_empty_dir() {
    let (root, _guard) = create_temp_dir("index-dry-run");
    let output = Command::new(rem_binary())
        .args(["index", "--dry-run"])
        .current_dir(&root)
        .output()
        .expect("failed to run rem index --dry-run");
    assert!(output.status.success());
}

#[test]
fn version_flag_shows_version_number() {
    let output = Command::new(rem_binary())
        .arg("--version")
        .output()
        .expect("failed to run rem --version");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.chars().any(|c| c.is_ascii_digit()), "stdout: {stdout}");
    assert!(stdout.contains('.'), "stdout: {stdout}");
}

#[test]
fn help_shows_completions_subcommand() {
    let output = Command::new(rem_binary())
        .arg("--help")
        .output()
        .expect("failed to run rem --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("completions"), "stdout: {stdout}");
}

#[test]
fn completions_bash_generates_output() {
    let output = Command::new(rem_binary())
        .args(["completions", "bash"])
        .output()
        .expect("failed to run rem completions bash");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("_rem"), "stdout: {stdout}");
    assert!(stdout.contains("complete"), "stdout: {stdout}");
}

#[test]
fn completions_fish_generates_output() {
    let output = Command::new(rem_binary())
        .args(["completions", "fish"])
        .output()
        .expect("failed to run rem completions fish");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("complete"), "stdout: {stdout}");
}

#[test]
fn completions_zsh_generates_output() {
    let output = Command::new(rem_binary())
        .args(["completions", "zsh"])
        .output()
        .expect("failed to run rem completions zsh");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("_rem"), "stdout: {stdout}");
}

#[test]
fn completions_powershell_generates_output() {
    let output = Command::new(rem_binary())
        .args(["completions", "powershell"])
        .output()
        .expect("failed to run rem completions powershell");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Register-ArgumentCompleter"), "stdout: {stdout}");
}

#[test]
fn completions_elvish_generates_output() {
    let output = Command::new(rem_binary())
        .args(["completions", "elvish"])
        .output()
        .expect("failed to run rem completions elvish");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "stdout should not be empty");
}
