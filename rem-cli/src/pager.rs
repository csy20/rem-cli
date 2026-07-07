use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::sync::RwLock;

/// Threshold for auto-paging (lines). Output shorter than this is printed directly.
/// Initialized to 50 by default; can be overridden via config or `init_page_threshold`.
static PAGE_THRESHOLD: LazyLock<RwLock<usize>> = LazyLock::new(|| RwLock::new(50));

/// Initialize the page threshold from config. Should be called at startup.
pub fn init_page_threshold(threshold: usize) {
    let mut t = PAGE_THRESHOLD.write().unwrap_or_else(|e| e.into_inner());
    *t = threshold;
}

/// Prints text to stdout, optionally piping through `less` if it's long
/// and a pager is available.
pub fn maybe_page(text: &str) {
    let threshold = *PAGE_THRESHOLD.read().unwrap_or_else(|e| e.into_inner());
    let line_count = text.lines().count();
    if line_count < threshold || !*PAGER_AVAILABLE {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pager_args_less() {
        let args = pager_args("less");
        assert_eq!(args, vec!["-R", "-F", "-X"]);
    }

    #[test]
    fn pager_args_busybox() {
        let args = pager_args("busybox");
        assert_eq!(args, vec!["-R", "-F", "-X"]);
    }

    #[test]
    fn pager_args_full_path_less() {
        let args = pager_args("/usr/bin/less");
        assert_eq!(args, vec!["-R", "-F", "-X"]);
    }

    #[test]
    fn pager_args_more() {
        let args = pager_args("more");
        assert!(args.is_empty());
    }

    #[test]
    fn pager_args_bat() {
        let args = pager_args("bat");
        assert!(args.is_empty());
    }

    #[test]
    fn pager_args_custom_pager() {
        let args = pager_args("/opt/bin/mypager");
        assert!(args.is_empty());
    }

    #[test]
    fn print_direct_ends_with_newline() {
        print_direct("hello\n");
    }

    #[test]
    fn print_direct_no_newline() {
        print_direct("hello");
    }

    #[test]
    fn maybe_page_short_text() {
        maybe_page("short text under threshold\n");
    }

    #[test]
    fn maybe_page_empty() {
        maybe_page("");
    }

    #[test]
    fn maybe_page_exact_threshold() {
        let text = (0..49).map(|i| format!("line_{}", i)).collect::<Vec<_>>().join("\n");
        maybe_page(&text);
    }
}
