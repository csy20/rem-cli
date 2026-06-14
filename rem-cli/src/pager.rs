use std::io::Write;
use std::process::{Command, Stdio};

/// Threshold for auto-paging (lines). Output shorter than this is printed directly.
const PAGE_THRESHOLD: usize = 50;

/// Prints text to stdout, optionally piping through `less` if it's long
/// and a pager is available.
pub fn maybe_page(text: &str) {
    let line_count = text.lines().count();
    if line_count < PAGE_THRESHOLD || !pager_available() {
        print_direct(text);
        return;
    }

    let mut child = match Command::new("less")
        .args(["-R", "-F", "-X"])
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

fn pager_available() -> bool {
    Command::new("less")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn print_direct(text: &str) {
    print!("{}", text);
    let _ = std::io::stdout().flush();
}
