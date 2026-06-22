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
        .args(["index", "--dir", root.to_str().unwrap()])
        .output()
        .expect("failed to run rem index on missing dir");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should handle gracefully — either success (fallback to cwd) or failure
    assert!(
        output.status.success() || stderr.contains("error"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
}
