//! Markdown rendering utilities.
//! Renders the ASCII-art welcome banner, markdown tables, and task lists.

use crate::ui::theme;

/// Renders the ASCII-art welcome banner lines.
pub fn render_welcome(model: &str, mode: &str, version: &str) -> Vec<String> {
    let t = theme::active();
    let mut lines = Vec::new();

    let top = format!(
        " {} {}",
        theme::paint(&t, "accent", "\u{256D}", true),
        theme::paint(
            &t,
            "text_faint",
            &format!("\u{2500}{} rem v{version} \u{2500}", "\u{2500}".repeat(20)),
            false
        )
    );
    lines.push(top);

    let mid = format!(
        " {} {:>3} {} {}",
        theme::paint(&t, "accent", "\u{2502}", true),
        "",
        theme::paint(&t, "accent", model, false),
        theme::paint_chip(&t, mode),
    );
    lines.push(mid);

    let cmd_hint = format!(
        "{}/{} {}  {} {} {}  {} {}",
        theme::paint(&t, "text_faint", "/", false),
        theme::paint(&t, "text_muted", "help", false),
        theme::paint(&t, "text_faint", "for commands", false),
        theme::paint(&t, "text_faint", "/", false),
        theme::paint(&t, "text_muted", "provider", false),
        theme::paint(&t, "text_faint", "to switch", false),
        theme::paint(&t, "text_faint", "/", false),
        theme::paint(&t, "text_muted", "theme", false),
    );
    let bot = format!(
        " {} {} {}",
        theme::paint(&t, "accent", "\u{2570}", true),
        theme::paint(&t, "text_faint", &format!("\u{2500}{}", "\u{2500}".repeat(46)), false),
        cmd_hint,
    );
    lines.push(bot);

    lines
}

/// Post-processes text to render markdown tables and task lists.
/// Tables receive column-aligned formatting; task list markers are replaced
/// with styled Unicode alternatives.
pub fn render_markdown(text: &str, t: &crate::ui::theme::Theme) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < lines.len() {
        if is_table_row(lines[i]) && i + 1 < lines.len() && is_table_separator(lines[i + 1]) {
            let table_end = collect_table(&lines, i);
            out.push_str(&render_table(&lines[i..table_end], t));
            out.push('\n');
            i = table_end;
        } else {
            out.push_str(&render_task_line(lines[i], t));
            out.push('\n');
            i += 1;
        }
    }
    out
}

fn is_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.matches('|').count() >= 3
}

fn is_table_separator(line: &str) -> bool {
    line.trim()
        .strip_prefix('|')
        .and_then(|s| s.strip_suffix('|'))
        .is_some_and(|inner| {
            inner
                .split('|')
                .all(|col| col.trim().is_empty() || col.trim().chars().all(|c| c == '-' || c == ':'))
        })
}

fn collect_table(lines: &[&str], start: usize) -> usize {
    let mut end = start;
    while end < lines.len() && is_table_row(lines[end]) {
        end += 1;
    }
    end
}

fn parse_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let inner = trimmed
        .strip_prefix('|')
        .unwrap_or(trimmed)
        .strip_suffix('|')
        .unwrap_or(trimmed);
    inner.split('|').map(|s| s.trim().to_string()).collect()
}

fn column_widths(rows: &[Vec<String>]) -> Vec<usize> {
    let mut widths = Vec::new();
    for row in rows {
        for (i, col) in row.iter().enumerate() {
            let w = col.chars().count();
            if i >= widths.len() {
                widths.push(w);
            } else {
                widths[i] = widths[i].max(w);
            }
        }
    }
    widths
}

fn render_table(rows: &[&str], t: &crate::ui::theme::Theme) -> String {
    let parsed: Vec<Vec<String>> = rows.iter().map(|r| parse_table_row(r)).collect();
    let widths = column_widths(&parsed);
    let mut out = String::new();
    for (ri, row) in parsed.iter().enumerate() {
        if ri == 1 {
            // separator row — render a horizontal rule
            out.push_str(&format!("{} ", theme::paint(t, "text_faint", "|", false)));
            for w in &widths {
                out.push_str(&theme::paint(
                    t,
                    "text_faint",
                    &format!("{:-<w$}|", "", w = w + 1),
                    false,
                ));
            }
            out.push('\n');
            continue;
        }
        out.push_str(&format!("{} ", theme::paint(t, "text_faint", "|", false)));
        for (ci, col) in row.iter().enumerate() {
            let w = widths.get(ci).copied().unwrap_or(0);
            let padded = format!(" {:<w$} ", col, w = w);
            if ri == 0 {
                out.push_str(&theme::paint(t, "text_bright", &padded, false));
            } else {
                out.push_str(&padded);
            }
            out.push_str(&theme::paint(t, "text_faint", "|", false));
        }
        out.push('\n');
    }
    out
}

fn render_task_line(line: &str, t: &crate::ui::theme::Theme) -> String {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    let prefix = &line[..indent];
    if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
        format!(
            "{}{} {}",
            prefix,
            theme::paint(t, "text_muted", "\u{25CB}", false),
            rest
        )
    } else if let Some(rest) = trimmed.strip_prefix("- [x] ") {
        format!("{}{} {}", prefix, theme::paint(t, "accent", "\u{2713}", false), rest)
    } else if let Some(rest) = trimmed.strip_prefix("- [X] ") {
        format!("{}{} {}", prefix, theme::paint(t, "accent", "\u{2713}", false), rest)
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcome_includes_model_and_mode() {
        let lines = render_welcome("gpt-4", "CHAT", "0.1.0");
        assert!(lines.len() == 3);
        assert!(lines.iter().any(|l| l.contains("gpt-4")));
    }

    #[test]
    fn detects_table_row() {
        assert!(is_table_row("| a | b |"));
        assert!(!is_table_row("not a table"));
        assert!(!is_table_row("| only one pipe |"));
    }

    #[test]
    fn detects_table_separator() {
        assert!(is_table_separator("| --- | --- |"));
        assert!(is_table_separator("| :--- | ---: |"));
        assert!(!is_table_separator("| a | b |"));
    }

    #[test]
    fn parses_table_row() {
        let cols = parse_table_row("| a | b |");
        assert_eq!(cols, vec!["a", "b"]);
    }

    #[test]
    fn renders_markdown_table() {
        let t = theme::active();
        let md = "| Col1 | Col2 |\n| --- | --- |\n| A | B |\n";
        let result = render_markdown(md, &t);
        assert!(result.contains("Col1"));
        assert!(result.contains("Col2"));
        assert!(result.contains("A"));
        assert!(result.contains("B"));
    }

    #[test]
    fn renders_task_list_checked() {
        let t = theme::active();
        let result = render_markdown("- [x] done", &t);
        assert!(!result.contains("- [x]"));
    }

    #[test]
    fn renders_task_list_unchecked() {
        let t = theme::active();
        let result = render_markdown("- [ ] todo", &t);
        assert!(!result.contains("- [ ]"));
    }

    #[test]
    fn passes_through_plain_text() {
        let t = theme::active();
        let result = render_markdown("hello world", &t);
        assert_eq!(result.trim(), "hello world");
    }
}
