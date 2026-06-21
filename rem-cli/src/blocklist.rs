//! Command sanitization and blocking.
//! Prevents execution of dangerous commands and normalizes user input.

use std::collections::BTreeMap;

/// Normalizes a command string to catch obfuscation attempts.
/// Strips extra whitespace, shell escapes, and common tricks.
fn normalize_cmd(cmd: &str) -> String {
    let mut s = cmd
        .chars()
        .filter(|c| !c.is_control() && *c != '\\')
        .collect::<String>();
    let mut prev = String::new();
    loop {
        let next = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if next == prev {
            break;
        }
        prev = next.clone();
        s = next;
    }
    s.to_lowercase()
}

pub(crate) fn is_command_blocked(cmd: &str) -> bool {
    let normalized = normalize_cmd(cmd);
    if normalized.is_empty() {
        return false;
    }
    // Exact dangerous command patterns (after normalization)
    let blocked_patterns = [
        "rm -rf /",
        "rm -rf --no-preserve-root",
        "rm -rf /*",
        "rm -rf /.",
        "mkfs",
        "dd if=",
        ":(){:|:&};:",
        "shutdown",
        "reboot",
    ];
    for pat in &blocked_patterns {
        if normalized.contains(pat) {
            return true;
        }
    }
    // Destructive device writes
    if normalized.starts_with("dd ") && normalized.contains("of=") {
        let of_targets = ["/dev/sda", "/dev/nvme", "/dev/mmcblk", "/dev/vda", "/dev/hda"];
        if of_targets.iter().any(|t| normalized.contains(t)) {
            return true;
        }
    }
    // chmod 777 on system roots
    if normalized.starts_with("chmod 777")
        && normalized
            .split_whitespace()
            .any(|w| w == "/" || w.starts_with('/') && w.len() < 6)
    {
        return true;
    }
    // Destructive wget/curl to pipe to shell
    if (normalized.starts_with("wget ") || normalized.starts_with("curl ")) && normalized.contains("| sh") {
        return true;
    }
    // rm -rf targeting root or common critical dirs
    let words: Vec<&str> = normalized.split_whitespace().collect();
    if words.len() >= 3
        && (words[0] == "rm" || words[0] == "rmdir")
        && words
            .iter()
            .any(|w| *w == "/" || *w == "/." || *w == "/*" || *w == "/etc" || *w == "/boot" || *w == "/dev")
    {
        return true;
    }
    false
}

pub(crate) fn looks_like_shell_command(line: &str) -> bool {
    let first = line.split_whitespace().next().unwrap_or_default();
    matches!(
        first,
        "ls" | "pwd" | "cd" | "mkdir" | "cp" | "mv" | "touch" | "cat" | "echo" | "rm" | "find" | "grep"
    )
}

pub(crate) fn sanitize_commands(cmds: &[String]) -> Vec<&str> {
    let mut seen = BTreeMap::<String, ()>::new();
    let mut out = Vec::new();
    for cmd in cmds {
        let key = cmd.trim().to_string();
        if key.is_empty() || seen.contains_key(&key) {
            continue;
        }
        seen.insert(key.clone(), ());
        out.push(cmd.trim());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_dangerous_commands() {
        assert!(is_command_blocked("rm -rf /tmp"));
        assert!(is_command_blocked("shutdown now"));
        assert!(!is_command_blocked("ls -la"));
    }

    #[test]
    fn command_sanitization_dedups() {
        let input = vec![" ls ".to_string(), "ls".to_string(), "".to_string()];
        let out = sanitize_commands(&input);
        assert_eq!(out, vec!["ls"]);
    }
}
