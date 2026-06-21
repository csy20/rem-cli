//! Terminal syntax highlighting.
//! Applies ANSI color highlighting to HTML, CSS, and JavaScript code
//! for prettier display in the terminal during code reviews and diffs.

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
static RE_CSS_VAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(:\s*)([^;}{]+)").expect("invalid regex literal"));
static RE_CSS_COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(/\*.*?\*/)").expect("invalid regex literal"));
static RE_JS_KW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(const|let|var|function|return|if|else|for|while|class|import|export|from|async|await|try|catch|new|this|document|console|window)\b").expect("invalid regex literal")
});
static RE_JS_STR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"('[^']*'|"[^"]*"|`[^`]*`)"#).expect("invalid regex literal"));
static RE_JS_COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(//.*)").expect("invalid regex literal"));

/// Applies ANSI syntax highlighting to code based on language hint.
pub fn highlight_code(content: &str, lang_hint: &str) -> String {
    let lang = lang_hint.to_lowercase();
    if lang.contains("html") {
        highlight_html(content)
    } else if lang.contains("css") {
        highlight_css(content)
    } else if lang.contains("js") || lang.contains("javascript") || lang.contains("ts") || lang.contains("typescript") {
        highlight_js(content)
    } else {
        highlight_generic(content)
    }
}

fn highlight_html(code: &str) -> String {
    let t = ui::theme::active();
    let mut out = code.to_string();
    out = RE_HIGHLIGHT_COMMENT
        .replace_all(&out, |caps: &regex::Captures| ui::theme::paint_dim(&t, &caps[1]))
        .to_string();
    out = RE_HIGHLIGHT_HTML_TAG
        .replace_all(&out, |caps: &regex::Captures| {
            let tag = &caps[1];
            let inner = RE_HIGHLIGHT_ATTR
                .replace_all(tag, |ac: &regex::Captures| ui::theme::paint_success_label(&t, &ac[1]))
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
        .replace_all(&out, |caps: &regex::Captures| ui::theme::paint_dim(&t, &caps[1]))
        .to_string();
    out = RE_CSS_PROP
        .replace_all(&out, |caps: &regex::Captures| {
            format!("{}{}{}", &caps[1], ui::theme::paint_warning(&t, &caps[2]), &caps[3])
        })
        .to_string();
    out = RE_CSS_VAL
        .replace_all(&out, |caps: &regex::Captures| {
            format!("{}{}", &caps[1], ui::theme::paint_success_label(&t, caps[2].trim()))
        })
        .to_string();
    out
}

fn highlight_js(code: &str) -> String {
    let t = ui::theme::active();
    let mut out = code.to_string();
    out = RE_JS_COMMENT
        .replace_all(&out, |caps: &regex::Captures| ui::theme::paint_dim(&t, &caps[1]))
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

/// Heuristically detects language (html/css/js/rust/etc.) from code content.
pub fn detect_language_from_content(code: &str) -> &str {
    let first_line = code.trim().lines().next().unwrap_or("");
    if first_line.starts_with("<!") || first_line.starts_with("<") {
        return "html";
    }
    if first_line.starts_with("fn ")
        || first_line.starts_with("pub ")
        || first_line.starts_with("impl ")
        || first_line.starts_with("unsafe ")
        || first_line.starts_with("#[")
        || first_line.starts_with("struct ")
        || first_line.starts_with("enum ")
        || first_line.starts_with("trait ")
        || first_line.starts_with("use ")
        || first_line.starts_with("mod ")
        || first_line.starts_with("let ")
        || first_line.starts_with("const ")
        || first_line.starts_with("static ")
        || first_line.starts_with("async ")
        || first_line.starts_with("await ")
        || first_line.starts_with("type ")
        || first_line.starts_with("macro_rules!")
    {
        return "rust";
    }
    if first_line.starts_with("func ")
        || first_line.starts_with("package ")
        || first_line.starts_with("import \"")
        || first_line.starts_with("import (")
    {
        return "go";
    }
    if first_line.starts_with("def ")
        || first_line.starts_with("class ")
        || first_line.starts_with("import ")
        || first_line.starts_with("from ")
        || first_line.starts_with("print(")
    {
        return "python";
    }
    if first_line.starts_with("#include")
        || first_line.starts_with("int ")
        || first_line.starts_with("char ")
        || first_line.starts_with("void ")
        || first_line.starts_with("float ")
        || first_line.starts_with("double ")
        || first_line.starts_with("long ")
        || first_line.starts_with("short ")
        || first_line.starts_with("unsigned ")
        || first_line.starts_with("signed ")
        || first_line.starts_with("static ")
        || first_line.starts_with("extern ")
        || first_line.starts_with("typedef ")
        || first_line.starts_with("struct ")
        || first_line.starts_with("enum ")
        || first_line.starts_with("union ")
    {
        return "c";
    }
    if first_line.starts_with("const ")
        || first_line.starts_with("let ")
        || first_line.starts_with("function ")
        || first_line.starts_with("import ")
    {
        return "js";
    }
    if first_line.contains("{") && first_line.contains("}") && !first_line.contains("fn ") && !first_line.contains("=>")
    {
        return "css";
    }
    ""
}
