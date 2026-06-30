use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

/// Threshold for auto-paging (lines). Output shorter than this is printed directly.
const PAGE_THRESHOLD: usize = 50;

/// Prints text to stdout, optionally piping through `less` if it's long
/// and a pager is available.
pub fn maybe_page(text: &str) {
    let line_count = text.lines().count();
    if line_count < PAGE_THRESHOLD || !*PAGER_AVAILABLE {
        print_direct(text);
        return;
    }

    let pager_cmd = std::env::var("PAGER").unwrap_or_else(|_| "less".to_string());
    let pager_parts: Vec<&str> = pager_cmd.split_whitespace().collect();
    let mut child = match Command::new(pager_parts.first().copied().unwrap_or("less"))
        .args(&pager_parts[1..])
        .args(pager_args(&pager_cmd))
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => {
            print_direct(text);
            return;
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }

    let _ = child.wait();
}

fn pager_args(cmd: &str) -> Vec<&str> {
    // Check if the binary name (without path) is "less"
    let binary_name = cmd.rsplit('/').next().unwrap_or(cmd);
    if binary_name == "less" || binary_name == "busybox" {
        vec!["-R", "-F", "-X"]
    } else {
        vec![]
    }
}

/// Cached result of checking whether a pager is available on this system.
static PAGER_AVAILABLE: LazyLock<bool> = LazyLock::new(|| {
    let pager_cmd = std::env::var("PAGER").unwrap_or_else(|_| "less".to_string());
    let pager_parts: Vec<&str> = pager_cmd.split_whitespace().collect();
    // Use --help which most pagers support (more portable than --version)
    Command::new(pager_parts.first().copied().unwrap_or("less"))
        .args(&pager_parts[1..])
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
});

fn print_direct(text: &str) {
    if text.ends_with('\n') {
        print!("{text}");
    } else {
        println!("{text}");
    }
    let _ = std::io::stdout().flush();
}
