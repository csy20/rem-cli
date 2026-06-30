//! Terminal syntax highlighting.
//! Applies ANSI color highlighting to HTML, CSS, and JavaScript code
//! for prettier display in the terminal during code reviews and diffs.

use std::fmt::Write;
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
    // Apply values BEFORE properties so the property regex doesn't match
    // within ANSI-colored values (values don't contain property-like patterns).
    out = RE_CSS_VAL
        .replace_all(&out, |caps: &regex::Captures| {
            let mut s = String::new();
            let _ = write!(s, "{}{}", &caps[1], ui::theme::paint_success_label(&t, caps[2].trim()));
            s
        })
        .to_string();
    out = RE_CSS_PROP
        .replace_all(&out, |caps: &regex::Captures| {
            let mut s = String::new();
            let _ = write!(s, "{}{}{}", &caps[1], ui::theme::paint_warning(&t, &caps[2]), &caps[3]);
            s
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
    // Rust-specific keywords (checked first for accuracy)
    if first_line.starts_with("fn ")
        || first_line.starts_with("pub ")
        || first_line.starts_with("impl ")
        || first_line.starts_with("unsafe ")
        || first_line.starts_with("#[")
        || first_line.starts_with("trait ")
        || first_line.starts_with("mod ")
        || first_line.starts_with("use ")
        || first_line.starts_with("macro_rules!")
        || first_line.starts_with("let ")
        || first_line.starts_with("async ")
        || first_line.starts_with("await ")
        || first_line.starts_with("type ")
        || first_line.starts_with("static ")
    {
        return "rust";
    }
    // JS/TS module import/export (check before Python's `import`)
    if first_line.starts_with("import {")
        || first_line.starts_with("import *")
        || first_line.starts_with("import type")
        || first_line.starts_with("export ")
    {
        return "js";
    }
    if first_line.starts_with("func ")
        || first_line.starts_with("package ")
        || first_line.starts_with("import \"")
        || first_line.starts_with("import (")
    {
        return "go";
    }
    // Python: `def`, `print(`, `from`, `class X:`, `import ` with `;` absence
    if first_line.starts_with("def ")
        || first_line.starts_with("print(")
        || first_line.starts_with("from ")
        || first_line.starts_with("class ") && first_line.trim_end().ends_with(':')
        || first_line.starts_with("import ") && !first_line.contains("{")
    {
        return "python";
    }
    // C/C++ keywords
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
        || first_line.starts_with("extern ")
        || first_line.starts_with("typedef ")
        || first_line.starts_with("union ")
    {
        return "c";
    }
    // Rust const (checked after C since `const` is rare in C headers)
    if first_line.starts_with("const ") {
        return "rust";
    }
    // Rust keywords that overlap with C (checked after C for disambiguation)
    if first_line.starts_with("struct ") && first_line.contains('{')
        || first_line.starts_with("enum ") && first_line.contains('{')
        || first_line.starts_with("union ") && first_line.contains('{')
        || first_line.starts_with("struct ") && first_line.trim_end().ends_with('{')
        || first_line.starts_with("enum ") && first_line.trim_end().ends_with('{')
        || first_line.starts_with("union ") && first_line.trim_end().ends_with('{')
    {
        return "rust";
    }
    // Fall through to C if struct/enum/union without braces (C style)
    if first_line.starts_with("struct ") || first_line.starts_with("enum ") || first_line.starts_with("union ") {
        return "c";
    }
    // JS/TS general patterns (checked after more specific keywords)
    if first_line.starts_with("function ") {
        return "js";
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_html_from_doctype() {
        assert_eq!(detect_language_from_content("<!DOCTYPE html>"), "html");
    }

    #[test]
    fn detect_html_from_tag() {
        assert_eq!(detect_language_from_content("<div>"), "html");
    }

    #[test]
    fn detect_rust_from_fn() {
        assert_eq!(detect_language_from_content("fn main() {"), "rust");
    }

    #[test]
    fn detect_rust_from_pub() {
        assert_eq!(detect_language_from_content("pub fn foo() {"), "rust");
    }

    #[test]
    fn detect_rust_from_impl() {
        assert_eq!(detect_language_from_content("impl Foo {"), "rust");
    }

    #[test]
    fn detect_rust_from_struct() {
        assert_eq!(detect_language_from_content("struct Point {"), "rust");
    }

    #[test]
    fn detect_rust_from_use() {
        assert_eq!(detect_language_from_content("use std::collections;"), "rust");
    }

    #[test]
    fn detect_rust_from_hash_bracket() {
        assert_eq!(detect_language_from_content("#[derive(Debug)]"), "rust");
    }

    #[test]
    fn detect_rust_from_unsafe() {
        assert_eq!(detect_language_from_content("unsafe {"), "rust");
    }

    #[test]
    fn detect_rust_from_enum() {
        assert_eq!(detect_language_from_content("enum Color {"), "rust");
    }

    #[test]
    fn detect_rust_from_trait() {
        assert_eq!(detect_language_from_content("trait Animal {"), "rust");
    }

    #[test]
    fn detect_rust_from_mod() {
        assert_eq!(detect_language_from_content("mod foo;"), "rust");
    }

    #[test]
    fn detect_rust_from_let() {
        assert_eq!(detect_language_from_content("let x = 5;"), "rust");
    }

    #[test]
    fn detect_rust_from_const() {
        assert_eq!(detect_language_from_content("const MAX: usize = 100;"), "rust");
    }

    #[test]
    fn detect_rust_from_static() {
        assert_eq!(detect_language_from_content("static NAME: &str = \"hello\";"), "rust");
    }

    #[test]
    fn detect_rust_from_async() {
        assert_eq!(detect_language_from_content("async fn fetch() {"), "rust");
    }

    #[test]
    fn detect_rust_from_type() {
        assert_eq!(
            detect_language_from_content("type Result<T> = std::result::Result<T, Error>;"),
            "rust"
        );
    }

    #[test]
    fn detect_rust_from_macro_rules() {
        assert_eq!(detect_language_from_content("macro_rules! vec {"), "rust");
    }

    #[test]
    fn detect_go_from_func() {
        assert_eq!(detect_language_from_content("func main() {"), "go");
    }

    #[test]
    fn detect_go_from_package() {
        assert_eq!(detect_language_from_content("package main"), "go");
    }

    #[test]
    fn detect_go_from_import_string() {
        assert_eq!(detect_language_from_content("import \"fmt\""), "go");
    }

    #[test]
    fn detect_python_from_def() {
        assert_eq!(detect_language_from_content("def hello():"), "python");
    }

    #[test]
    fn detect_python_from_class() {
        assert_eq!(detect_language_from_content("class MyClass:"), "python");
    }

    #[test]
    fn detect_python_from_print() {
        assert_eq!(detect_language_from_content("print(\"hello\")"), "python");
    }

    #[test]
    fn detect_c_from_include() {
        assert_eq!(detect_language_from_content("#include <stdio.h>"), "c");
    }

    #[test]
    fn detect_c_from_int() {
        assert_eq!(detect_language_from_content("int main() {"), "c");
    }

    #[test]
    fn detect_c_from_void() {
        assert_eq!(detect_language_from_content("void foo() {"), "c");
    }

    #[test]
    fn detect_js_from_function() {
        assert_eq!(detect_language_from_content("function foo() {"), "js");
    }

    #[test]
    fn detect_empty_string() {
        assert_eq!(detect_language_from_content(""), "");
    }

    #[test]
    fn detect_whitespace() {
        assert_eq!(detect_language_from_content("   "), "");
    }

    #[test]
    fn highlight_html_wraps_in_colored_output() {
        let result = highlight_code("<div class=\"foo\">hello</div>", "html");
        assert!(result.contains("hello"));
        assert!(result.contains("div"));
    }

    #[test]
    fn highlight_css_wraps_in_colored_output() {
        let result = highlight_code("body { color: red; }", "css");
        assert!(result.contains("body"));
        assert!(result.contains("color"));
    }

    #[test]
    fn highlight_js_wraps_in_colored_output() {
        let result = highlight_code("const x = 1;", "js");
        assert!(result.contains("const"));
        assert!(result.contains("x"));
    }

    #[test]
    fn highlight_typescript_wraps_in_colored_output() {
        let result = highlight_code("const x: number = 1;", "typescript");
        assert!(result.contains("const"));
    }

    #[test]
    fn highlight_generic_returns_unchanged() {
        let code = "some random code without known language";
        let result = highlight_code(code, "unknown");
        assert_eq!(result, code);
    }

    #[test]
    fn highlight_empty_string() {
        let result = highlight_code("", "html");
        assert_eq!(result, "");
    }
}
