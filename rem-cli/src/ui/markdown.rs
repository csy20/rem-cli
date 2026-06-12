use crate::ui::theme;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcome_includes_model_and_mode() {
        let lines = render_welcome("gpt-4", "CHAT", "0.1.0");
        assert!(lines.len() == 3);
        assert!(lines.iter().any(|l| l.contains("gpt-4")));
    }
}
