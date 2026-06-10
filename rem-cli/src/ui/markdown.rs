/// Markdown-to-ANSI renderer for model responses.
/// Parses common markdown and renders inline using the active theme.
/// No external dependencies — pure string processing + ANSI codes.
use crate::ui::theme;

#[derive(Debug, PartialEq)]
enum BlockType {
    Paragraph,
    H1,
    H2,
    H3,
    H4,
    CodeFence(String), // language label
    InlineCode(String),
    Quote,
    BulletList,
    OrderedList(usize), // starting number
    ThematicBreak,
    Empty,
}

/// Render a full markdown response to formatted ANSI strings.
/// Returns a Vec of lines (with ANSI escapes) ready to print.
pub fn render(text: &str) -> Vec<String> {
    let t = theme::active();
    let mut out: Vec<String> = Vec::new();
    let mut in_fence = false;
    let mut fence_lang = String::new();
    let mut fence_lines: Vec<String> = Vec::new();

    for raw_line in text.lines() {
        if in_fence {
            if raw_line.trim_start().starts_with("```") {
                // End code block
                let header_line = format!(
                    "  {}",
                    t.section_header(&if fence_lang.is_empty() {
                        "code".into()
                    } else {
                        fence_lang.clone()
                    })
                );
                out.push(header_line);

                // Flush fence lines with gutter
                let gutter = t.code_gutter();
                for code_line in &fence_lines {
                    let rendered = format!("  {gutter} {code_line}");
                    out.push(rendered);
                }
                out.push(format!("  {gutter}"));

                in_fence = false;
                fence_lang.clear();
                fence_lines.clear();
                continue;
            }
            fence_lines.push(raw_line.to_string());
            continue;
        }

        if raw_line.trim_start().starts_with("```") {
            in_fence = true;
            fence_lang = raw_line
                .trim_start()
                .trim_start_matches('`')
                .trim()
                .to_string();
            continue;
        }

        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            out.push(String::new());
            continue;
        }

        match classify_line(trimmed) {
            BlockType::H1 => {
                let text = trimmed.trim_start_matches('#').trim();
                out.push(format!("  {}", theme::paint(&t, "accent", text, true)));
            }
            BlockType::H2 => {
                let text = trimmed.trim_start_matches('#').trim();
                out.push(format!(
                    "  {}",
                    theme::paint(
                        &t,
                        "accent",
                        &format!("\u{2500}\u{2500} {} \u{2500}\u{2500}", text),
                        true
                    )
                ));
            }
            BlockType::H3 | BlockType::H4 => {
                let text = trimmed.trim_start_matches('#').trim();
                out.push(format!("  {}", theme::paint(&t, "accent_dim", text, true)));
            }
            BlockType::Quote => {
                let text = trimmed.trim_start_matches('>').trim();
                let bar = theme::paint(&t, "text_faint", "\u{2502}", false);
                let body = render_inline(text, false);
                out.push(format!("  {bar}  {body}"));
            }
            BlockType::BulletList => {
                let text = trimmed
                    .trim_start_matches(|c| c == '-' || c == '*' || c == ' ')
                    .trim();
                let dot = theme::paint(&t, "accent_dim", "\u{2022}", true);
                let body = render_inline(text, false);
                out.push(format!("  {dot}  {body}"));
            }
            BlockType::OrderedList(n) => {
                // Remove the number prefix
                let text = trimmed
                    .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')
                    .trim();
                let num = theme::paint(&t, "accent_dim", &format!("{}.", n), true);
                let body = render_inline(text, false);
                out.push(format!("  {num}  {body}"));
            }
            BlockType::ThematicBreak => {
                out.push(format!("  {}", t.divider()));
            }
            _ => {
                let body = render_inline(trimmed, false);
                if !body.is_empty() {
                    out.push(format!("  {body}"));
                }
            }
        }
    }

    // Flush remaining fence if unclosed
    if in_fence && !fence_lines.is_empty() {
        let gutter = t.code_gutter();
        for code_line in &fence_lines {
            out.push(format!("  {gutter} {code_line}"));
        }
        out.push(format!("  {gutter}"));
    }

    out
}

/// Render inline markdown elements (*bold*, `code`) in a single line.
fn render_inline(text: &str, _is_code: bool) -> String {
    let t = theme::active();
    let mut out = String::with_capacity(text.len() + 32);

    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next(); // consume second *
                                  // Read until **
                    let mut bold_text = String::new();
                    while let Some(next) = chars.next() {
                        if next == '*' && chars.peek() == Some(&'*') {
                            chars.next(); // consume closing **
                            break;
                        }
                        bold_text.push(next);
                    }
                    out.push_str(&theme::paint(&t, "accent", &bold_text, true));
                } else {
                    // Single * could be italic or just literal
                    // For simplicity, treat as accent dim
                    let mut italic_text = String::new();
                    while let Some(next) = chars.next() {
                        if next == '*' {
                            break;
                        }
                        italic_text.push(next);
                    }
                    out.push_str(&theme::paint(&t, "accent_dim", &italic_text, false));
                }
            }
            '`' => {
                let mut code_text = String::new();
                // Collect until backtick
                for ch in chars.by_ref() {
                    if ch == '`' {
                        break;
                    }
                    code_text.push(ch);
                }
                out.push_str(&theme::paint_on(
                    &t,
                    "accent_dim",
                    "code_bg",
                    &format!(" {} ", code_text),
                    false,
                ));
            }
            _ => {
                // Check for markdown links [text](url) — render text only
                if c == '[' {
                    let mut link_text = String::new();
                    let mut is_link = false;
                    let mut paren_count = 0;
                    for ch in chars.by_ref() {
                        if ch == ']' {
                            if chars.peek() == Some(&'(') {
                                chars.next(); // consume '('
                                is_link = true;
                                // skip url
                                for inner in chars.by_ref() {
                                    if inner == ')' {
                                        break;
                                    }
                                    paren_count += 1;
                                }
                            }
                            break;
                        }
                        link_text.push(ch);
                    }
                    if is_link {
                        out.push_str(&theme::paint(&t, "accent", &link_text, false));
                    } else {
                        out.push('[');
                        out.push_str(&link_text);
                        if paren_count > 0 {
                            out.push(']');
                            out.push('(');
                        }
                    }
                } else {
                    out.push(c);
                }
            }
        }
    }

    out
}

fn classify_line(trimmed: &str) -> BlockType {
    if trimmed.starts_with("### ") || trimmed.starts_with("###\t") {
        if trimmed.starts_with("#### ") {
            BlockType::H4
        } else {
            BlockType::H3
        }
    } else if trimmed.starts_with("## ") || trimmed.starts_with("##\t") {
        BlockType::H2
    } else if trimmed.starts_with("# ") || trimmed.starts_with("#\t") {
        BlockType::H1
    } else if trimmed.starts_with('>') {
        BlockType::Quote
    } else if trimmed == "---" || trimmed == "***" || trimmed == "___" {
        BlockType::ThematicBreak
    } else if let Some(stripped) = trimmed.strip_prefix("- ") {
        if !stripped.is_empty() {
            BlockType::BulletList
        } else {
            BlockType::Paragraph
        }
    } else if let Some(stripped) = trimmed.strip_prefix("* ") {
        if !stripped.is_empty() {
            BlockType::BulletList
        } else {
            BlockType::Paragraph
        }
    } else if let Some(num_str) = trimmed.split('.').next() {
        if let Ok(n) = num_str.parse::<usize>() {
            // Must be followed by a space
            let after = &trimmed[num_str.len()..];
            if after.starts_with(". ") {
                BlockType::OrderedList(n)
            } else {
                BlockType::Paragraph
            }
        } else {
            BlockType::Paragraph
        }
    } else {
        BlockType::Paragraph
    }
}

/// Render a "stats footer" line for the end of a response.
pub fn render_footer(model: &str, elapsed_s: f64, tokens: u32) -> String {
    let t = theme::active();
    let divider = theme::paint(&t, "text_faint", "\u{2500}", false).repeat(4);
    let model_tag = theme::paint_chip(&t, model);
    let dur = theme::paint_dim(&t, &format!("\u{23f1} {:.1}s", elapsed_s));
    let speed = if elapsed_s > 0.0 && tokens > 0 {
        let tps = tokens as f64 / elapsed_s;
        theme::paint_dim(&t, &format!("{:.0} tok/s", tps))
    } else {
        String::new()
    };
    let dot = theme::paint_dim(&t, "\u{00B7}");
    format!(" {divider}  {model_tag}  {dot}  {dur}  {speed}  {divider}")
}

/// Render a prompt arrow.
pub fn prompt_arrow() -> String {
    let t = theme::active();
    theme::paint(&t, "accent", "\u{203A}", true)
}

/// Render a welcome header shown once at session start.
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
        theme::paint(
            &t,
            "text_faint",
            &format!("\u{2500}{}", "\u{2500}".repeat(46)),
            false
        ),
        cmd_hint,
    );
    lines.push(bot);

    lines
}

/// Render a status bar line showing model, mode, message count, context.
pub fn render_status_bar(model: &str, mode: &str, msgs: usize, ctx_pct: u8) -> String {
    let t = theme::active();
    let dot = theme::paint(&t, "text_faint", "\u{00B7}", false);

    let model_val = theme::paint(&t, "text_muted", model, false);
    let mode_val = theme::paint(&t, "text_muted", mode, false);
    let msgs_val = theme::paint(
        &t,
        "text_muted",
        &format!("{} turn{}", msgs, if msgs == 1 { "" } else { "s" }),
        false,
    );
    let ctx_val = theme::paint(&t, "text_muted", &format!("ctx {}%", ctx_pct), false);

    let left = format!("  {model_val}  {dot}  {mode_val}  {dot}  {msgs_val}  {dot}  {ctx_val}");

    let slash = theme::paint(&t, "text_faint", "/", false);
    let help = theme::paint(&t, "text_muted", "commands", false);
    let q_hint = theme::paint(&t, "text_faint", "?", false);
    let q_help = theme::paint(&t, "text_muted", "help", false);
    let right = format!("{slash}  {help}    {q_hint}  {q_help}");

    let width = theme::visible_width();
    let left_len = theme::visible_len(&left);
    let right_len = theme::visible_len(&right);
    let pad = if width > left_len + right_len + 4 {
        " ".repeat(width - left_len - right_len - 2)
    } else {
        "    ".to_string()
    };

    format!("{left}{pad}{right}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_plain_text() {
        let lines = render("hello world");
        assert!(lines.iter().any(|l| l.contains("hello world")));
    }

    #[test]
    fn renders_bold_text() {
        let lines = render("this is **bold** text");
        assert!(lines.iter().any(|l| l.contains("bold")));
    }

    #[test]
    fn renders_inline_code() {
        let lines = render("use `foo()` to call it");
        assert!(lines.iter().any(|l| l.contains("foo()")));
    }

    #[test]
    fn renders_code_fence() {
        let lines = render("```rust\nfn main() {}\n```");
        assert!(lines.iter().any(|l| l.contains("rust")));
        assert!(lines.iter().any(|l| l.contains("fn main()")));
    }

    #[test]
    fn renders_header() {
        let lines = render("# Big Header\n## Sub Header\n### Small");
        assert!(lines.iter().any(|l| l.contains("Big Header")));
        assert!(lines.iter().any(|l| l.contains("Sub Header")));
    }

    #[test]
    fn renders_list() {
        let lines = render("- item one\n- item two");
        assert!(lines.iter().any(|l| l.contains("item one")));
        assert!(lines.iter().any(|l| l.contains("item two")));
    }

    #[test]
    fn renders_ordered_list() {
        let lines = render("1. first\n2. second");
        assert!(lines.iter().any(|l| l.contains("first")));
        assert!(lines.iter().any(|l| l.contains("second")));
    }

    #[test]
    fn render_unclosed_fence_does_not_panic() {
        let lines = render("```\nsome code\nno closing");
        assert!(!lines.is_empty());
    }

    #[test]
    fn footer_contains_model_and_timing() {
        let s = render_footer("test-model", 1.5, 100);
        assert!(s.contains("test-model"));
        assert!(s.contains("1.5"));
    }

    #[test]
    fn prompt_arrow_produces_unicode() {
        let s = prompt_arrow();
        assert!(s.contains('\u{203A}'));
    }

    #[test]
    fn status_bar_includes_model() {
        let s = render_status_bar("gpt-4", "CHAT", 3, 15);
        assert!(s.contains("gpt-4"));
        assert!(s.contains("CHAT"));
        assert!(s.contains("3 turns"));
        assert!(s.contains("ctx 15%"));
    }

    #[test]
    fn welcome_includes_model_and_mode() {
        let lines = render_welcome("gpt-4", "CHAT", "0.1.0");
        assert!(lines.len() == 3);
        assert!(lines.iter().any(|l| l.contains("gpt-4")));
    }
}
