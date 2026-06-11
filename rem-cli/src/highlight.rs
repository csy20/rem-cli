use std::sync::LazyLock;

use regex::Regex;

use crate::ui;

static RE_HIGHLIGHT_HTML_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(</?\w+[^>]*>)").expect("invalid regex literal"));
static RE_HIGHLIGHT_ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"("[^"]*")"#).expect("invalid regex literal"));
static RE_HIGHLIGHT_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(<!--.*?-->)").expect("invalid regex literal"));
static RE_CSS_PROP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^(\s*)([a-zA-Z-]+)(\s*:)").expect("invalid regex literal"));
static RE_CSS_VAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(:\s*)([^;}{]+)").expect("invalid regex literal"));
static RE_CSS_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(/\*.*?\*/)").expect("invalid regex literal"));
static RE_JS_KW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(const|let|var|function|return|if|else|for|while|class|import|export|from|async|await|try|catch|new|this|document|console|window)\b").expect("invalid regex literal")
});
static RE_JS_STR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"('[^']*'|"[^"]*"|`[^`]*`)"#).expect("invalid regex literal"));
static RE_JS_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(//.*)").expect("invalid regex literal"));

pub fn highlight_code(content: &str, lang_hint: &str) -> String {
    let lang = lang_hint.to_lowercase();
    if lang.contains("html") {
        highlight_html(content)
    } else if lang.contains("css") {
        highlight_css(content)
    } else if lang.contains("js")
        || lang.contains("javascript")
        || lang.contains("ts")
        || lang.contains("typescript")
    {
        highlight_js(content)
    } else {
        highlight_generic(content)
    }
}

fn highlight_html(code: &str) -> String {
    let t = ui::theme::active();
    let mut out = code.to_string();
    out = RE_HIGHLIGHT_COMMENT
        .replace_all(&out, |caps: &regex::Captures| {
            ui::theme::paint_dim(&t, &caps[1])
        })
        .to_string();
    out = RE_HIGHLIGHT_HTML_TAG
        .replace_all(&out, |caps: &regex::Captures| {
            let tag = &caps[1];
            let inner = RE_HIGHLIGHT_ATTR
                .replace_all(tag, |ac: &regex::Captures| {
                    ui::theme::paint_success_label(&t, &ac[1])
                })
                .to_string();
            ui::theme::paint(&t, "accent", &inner, true)
        })
        .to_string();
    out
}

fn highlight_css(code: &str) -> String {
    let t = ui::theme::active();
    let mut out = code.to_string();
    out = RE_CSS_COMMENT
        .replace_all(&out, |caps: &regex::Captures| {
            ui::theme::paint_dim(&t, &caps[1])
        })
        .to_string();
    out = RE_CSS_PROP
        .replace_all(&out, |caps: &regex::Captures| {
            format!(
                "{}{}{}",
                &caps[1],
                ui::theme::paint_warning(&t, &caps[2]),
                &caps[3]
            )
        })
        .to_string();
    out = RE_CSS_VAL
        .replace_all(&out, |caps: &regex::Captures| {
            format!(
                "{}{}",
                &caps[1],
                ui::theme::paint_success_label(&t, &caps[2].trim())
            )
        })
        .to_string();
    out
}

fn highlight_js(code: &str) -> String {
    let t = ui::theme::active();
    let mut out = code.to_string();
    out = RE_JS_COMMENT
        .replace_all(&out, |caps: &regex::Captures| {
            ui::theme::paint_dim(&t, &caps[1])
        })
        .to_string();
    out = RE_JS_STR
        .replace_all(&out, |caps: &regex::Captures| {
            ui::theme::paint_success_label(&t, &caps[1])
        })
        .to_string();
    out = RE_JS_KW
        .replace_all(&out, |caps: &regex::Captures| {
            ui::theme::paint(&t, "accent_info", &caps[1], true)
        })
        .to_string();
    out
}

fn highlight_generic(code: &str) -> String {
    code.to_string()
}

pub fn detect_language_from_content(code: &str) -> &str {
    let first_line = code.trim().lines().next().unwrap_or("");
    if first_line.starts_with("<!") || first_line.starts_with("<") {
        "html"
    } else if first_line.contains("{")
        && first_line.contains("}")
        && !first_line.contains("function")
        && !first_line.contains("=>")
    {
        "css"
    } else if first_line.starts_with("const ")
        || first_line.starts_with("let ")
        || first_line.starts_with("function ")
        || first_line.starts_with("import ")
    {
        "js"
    } else {
        ""
    }
}
