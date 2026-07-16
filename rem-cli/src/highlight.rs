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

/// Applies ANSI syntax highlighting to code based on language hint.
pub fn highlight_code(content: &str, lang_hint: &str) -> String {
    let lang = lang_hint.to_lowercase();
    if lang.contains("html") {
        highlight_html(content)
    } else if lang.contains("css") {
        highlight_css(content)
    } else if lang.contains("js") || lang.contains("javascript") || lang.contains("ts") || lang.contains("typescript") {
        highlight_js(content)
    } else if lang.contains("rust") || lang.contains("rs") {
        highlight_rust(content)
    } else if lang.contains("python") || lang.contains("py") {
        highlight_python(content)
    } else if lang.contains("go") || lang == "golang" {
        highlight_go(content)
    } else if lang.contains("json") {
        highlight_json(content)
    } else if lang.contains("cpp") || lang == "c++" || lang == "cxx" || lang == "c" || lang == "h" {
        highlight_c(content)
    } else if lang.contains("java") {
        highlight_java(content)
    } else if lang.contains("ruby") || lang.contains("rb") {
        highlight_ruby(content)
    } else if lang.contains("php") {
        highlight_php(content)
    } else if lang.contains("bash") || lang.contains("sh") || lang.contains("shell") || lang.contains("zsh") {
        highlight_bash(content)
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
    static RE_JS_ALL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(//[^\n]*)|(\b(?:const|let|var|function|return|if|else|for|while|class|import|export|from|async|await|try|catch|new|this|document|console|window)\b)|('[^']*'|"[^"]*"|`[^`]*`)"#).expect("invalid js all regex")
    });
    RE_JS_ALL
        .replace_all(code, |caps: &regex::Captures| {
            if let Some(m) = caps.get(1) {
                ui::theme::paint_dim(&t, m.as_str())
            } else if let Some(m) = caps.get(2) {
                ui::theme::paint(&t, "accent_info", m.as_str(), true)
            } else if let Some(m) = caps.get(3) {
                ui::theme::paint_success_label(&t, m.as_str())
            } else {
                caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default()
            }
        })
        .to_string()
}

fn highlight_rust(code: &str) -> String {
    let t = ui::theme::active();
    static RE_RUST_ALL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(//[^\n]*)|(\b(?:fn|let|mut|pub|use|mod|struct|enum|trait|impl|match|if|else|for|while|loop|return|async|await|ref|where|type|const|static|unsafe|dyn|self|super|crate|true|false|Some|None|Ok|Err|Result|Option|Box|String|Vec|HashMap)\b)|("[^"]*"|'[^']*')"#).expect("invalid rust all regex")
    });
    RE_RUST_ALL
        .replace_all(code, |caps: &regex::Captures| {
            if let Some(m) = caps.get(1) {
                ui::theme::paint_dim(&t, m.as_str())
            } else if let Some(m) = caps.get(2) {
                ui::theme::paint(&t, "accent_info", m.as_str(), true)
            } else if let Some(m) = caps.get(3) {
                ui::theme::paint_success_label(&t, m.as_str())
            } else {
                caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default()
            }
        })
        .to_string()
}

fn highlight_python(code: &str) -> String {
    let t = ui::theme::active();
    static RE_PY_ALL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(#[^\n]*)|(\b(?:def|class|return|if|elif|else|for|while|import|from|as|try|except|finally|with|pass|break|continue|and|or|not|in|is|None|True|False|raise|yield|lambda|self|async|await|print|len|range|map|filter|type|open|super|del|global|nonlocal|assert|match|case)\b)|('''[^']*'''|"""[^"]*"""|'[^']*'|"[^"]*")"#).expect("invalid python regex")
    });
    RE_PY_ALL
        .replace_all(code, |caps: &regex::Captures| {
            if let Some(m) = caps.get(1) {
                ui::theme::paint_dim(&t, m.as_str())
            } else if let Some(m) = caps.get(2) {
                ui::theme::paint(&t, "accent_info", m.as_str(), true)
            } else if let Some(m) = caps.get(3) {
                ui::theme::paint_success_label(&t, m.as_str())
            } else {
                caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default()
            }
        })
        .to_string()
}

fn highlight_go(code: &str) -> String {
    let t = ui::theme::active();
    static RE_GO_ALL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(//[^\n]*|/\*.*?\*/)|(\b(?:func|package|import|return|if|else|for|range|switch|case|default|break|continue|var|const|type|struct|interface|map|chan|go|defer|select|nil|true|false|make|new|len|cap|append|close|panic|recover|error|string|int|bool|float64|byte|rune|uint|int64|float32|complex64|complex128|uintptr|int8|int16|int32|int64|uint8|uint16|uint32|uint64)\b)|("[^"]*"|`[^`]*`)"#).expect("invalid go regex")
    });
    RE_GO_ALL
        .replace_all(code, |caps: &regex::Captures| {
            if let Some(m) = caps.get(1) {
                ui::theme::paint_dim(&t, m.as_str())
            } else if let Some(m) = caps.get(2) {
                ui::theme::paint(&t, "accent_info", m.as_str(), true)
            } else if let Some(m) = caps.get(3) {
                ui::theme::paint_success_label(&t, m.as_str())
            } else {
                caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default()
            }
        })
        .to_string()
}

fn highlight_json(code: &str) -> String {
    let t = ui::theme::active();
    static RE_JSON_ALL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"("[^"]*"\s*:)|("[^"]*")|(\btrue\b|\bfalse\b|\bnull\b)"#).expect("invalid json regex")
    });
    RE_JSON_ALL
        .replace_all(code, |caps: &regex::Captures| {
            if let Some(m) = caps.get(1) {
                ui::theme::paint(&t, "accent_info", m.as_str(), true).to_string()
            } else if let Some(m) = caps.get(2) {
                ui::theme::paint_success_label(&t, m.as_str())
            } else if let Some(m) = caps.get(3) {
                ui::theme::paint(&t, "accent", m.as_str(), true)
            } else {
                caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default()
            }
        })
        .to_string()
}

fn highlight_c(code: &str) -> String {
    let t = ui::theme::active();
    static RE_C_ALL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(//[^\n]*|/\*.*?\*/)|(\b(?:int|char|void|float|double|long|short|unsigned|signed|struct|union|enum|typedef|const|static|extern|volatile|register|auto|sizeof|return|if|else|for|while|do|switch|case|default|break|continue|goto|include|define|ifdef|ifndef|endif|pragma|error|NULL|true|false|size_t|ssize_t|uint8_t|uint16_t|uint32_t|uint64_t|int8_t|int16_t|int32_t|int64_t|bool|printf|scanf|malloc|calloc|realloc|free|fopen|fclose|fread|fwrite|fprintf|fscanf|fgets|fputs|fgetc|fputc|fseek|ftell|rewind|fprintf|sprintf|snprintf|strlen|strcpy|strcat|strcmp|strchr|strstr|memcpy|memmove|memset|memcmp|assert|FILE|NULL|EOF)\b)|("[^"]*"|'[^']*')"#).expect("invalid c regex")
    });
    RE_C_ALL
        .replace_all(code, |caps: &regex::Captures| {
            if let Some(m) = caps.get(1) {
                ui::theme::paint_dim(&t, m.as_str())
            } else if let Some(m) = caps.get(2) {
                ui::theme::paint(&t, "accent_info", m.as_str(), true)
            } else if let Some(m) = caps.get(3) {
                ui::theme::paint_success_label(&t, m.as_str())
            } else {
                caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default()
            }
        })
        .to_string()
}

fn highlight_java(code: &str) -> String {
    let t = ui::theme::active();
    static RE_JAVA_ALL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(//[^\n]*|/\*.*?\*/)|(\b(?:public|private|protected|static|final|class|interface|enum|extends|implements|abstract|synchronized|volatile|transient|native|strictfp|package|import|new|this|super|return|if|else|for|while|do|switch|case|default|break|continue|try|catch|finally|throw|throws|instanceof|boolean|byte|short|int|long|float|double|char|void|null|true|false|String|System|Math|List|Map|Set|ArrayList|HashMap|HashSet|Optional|Integer|Double|Boolean|Object|Exception|RuntimeException|Error|Thread|Runnable|Override|Deprecated|SuppressWarnings)\b)|("[^"]*"|'[^']*')"#).expect("invalid java regex")
    });
    RE_JAVA_ALL
        .replace_all(code, |caps: &regex::Captures| {
            if let Some(m) = caps.get(1) {
                ui::theme::paint_dim(&t, m.as_str())
            } else if let Some(m) = caps.get(2) {
                ui::theme::paint(&t, "accent_info", m.as_str(), true)
            } else if let Some(m) = caps.get(3) {
                ui::theme::paint_success_label(&t, m.as_str())
            } else {
                caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default()
            }
        })
        .to_string()
}

fn highlight_ruby(code: &str) -> String {
    let t = ui::theme::active();
    static RE_RUBY_ALL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(#[^\n]*|^=begin.*?^=end)|(\b(?:def|class|module|end|if|elsif|else|unless|case|when|then|for|while|until|do|yield|return|break|next|redo|retry|raise|rescue|ensure|begin|nil|true|false|self|super|and|or|not|in|is_a|respond_to|attr_accessor|attr_reader|attr_writer|require|include|extend|load|private|public|protected|lambda|proc|block_given|each|map|select|reject|reduce|inject|puts|print|p|require_relative|defined|catch|throw|fail|alias|undef)\b)|("[^"]*"|'[^']*'|`[^`]*`)"#).expect("invalid ruby regex")
    });
    RE_RUBY_ALL
        .replace_all(code, |caps: &regex::Captures| {
            if let Some(m) = caps.get(1) {
                ui::theme::paint_dim(&t, m.as_str())
            } else if let Some(m) = caps.get(2) {
                ui::theme::paint(&t, "accent_info", m.as_str(), true)
            } else if let Some(m) = caps.get(3) {
                ui::theme::paint_success_label(&t, m.as_str())
            } else {
                caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default()
            }
        })
        .to_string()
}

fn highlight_php(code: &str) -> String {
    let t = ui::theme::active();
    static RE_PHP_ALL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(//[^\n]*|/\*.*?\*/|#[^\n]*)|(\b(?:echo|print|die|exit|include|include_once|require|require_once|class|function|return|if|else|elseif|for|foreach|while|do|switch|case|default|break|continue|try|catch|finally|throw|new|this|self|parent|public|private|protected|static|final|abstract|interface|implements|extends|trait|use|namespace|const|var|true|false|null|isset|unset|empty|array|list|as|or|and|xor|instanceof|clone|declare|global|goto|match|fn|int|float|string|bool|void|mixed|never|iterable|callable|array|object|resource|numeric)\b)|("[^"]*"|'[^']*'|`[^`]*`)"#).expect("invalid php regex")
    });
    RE_PHP_ALL
        .replace_all(code, |caps: &regex::Captures| {
            if let Some(m) = caps.get(1) {
                ui::theme::paint_dim(&t, m.as_str())
            } else if let Some(m) = caps.get(2) {
                ui::theme::paint(&t, "accent_info", m.as_str(), true)
            } else if let Some(m) = caps.get(3) {
                ui::theme::paint_success_label(&t, m.as_str())
            } else {
                caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default()
            }
        })
        .to_string()
}

fn highlight_bash(code: &str) -> String {
    let t = ui::theme::active();
    static RE_BASH_ALL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r##"(#[^\n]*)|(\b(?:if|then|else|elif|fi|for|while|do|done|case|esac|in|function|return|exit|break|continue|export|local|source|shift|read|echo|printf|set|unset|declare|typeset|alias|unalias|trap|exec|eval|select|until|cd|ls|mkdir|rm|cp|mv|chmod|chown|grep|sed|awk|cat|find|sort|uniq|wc|head|tail|cut|tr|tee|diff|patch|tar|gzip|gunzip|zip|unzip|make|cmake|npm|yarn|node|python|python3|pip|ruby|gem|cargo|rustc|go|docker|git|curl|wget|xargs|kill|ps|top|htop|systemctl|journalctl|sudo|su|which|whereis|whoami|env|true|false|yes|no|test|let|EQ|NE|LT|GT|LE|GE|Z|N|O)\b)|("[^"]*"|'[^']*'|`[^`]*`)"##).expect("invalid bash regex")
    });
    RE_BASH_ALL
        .replace_all(code, |caps: &regex::Captures| {
            if let Some(m) = caps.get(1) {
                ui::theme::paint_dim(&t, m.as_str())
            } else if let Some(m) = caps.get(2) {
                ui::theme::paint(&t, "accent_info", m.as_str(), true)
            } else if let Some(m) = caps.get(3) {
                ui::theme::paint_success_label(&t, m.as_str())
            } else {
                caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default()
            }
        })
        .to_string()
}

fn highlight_generic(code: &str) -> String {
    code.to_string()
}

/// Heuristically detects language (html/css/js/rust/etc.) from code content.
pub fn detect_language_from_content(code: &str) -> &str {
    let first_line = code.trim().lines().next().unwrap_or("");

    // Shebang-based detection (must be first — unambiguous)
    if first_line.starts_with("#!/") {
        if first_line.contains("bash") || first_line.contains("sh") || first_line.contains("zsh") {
            return "bash";
        }
        if first_line.contains("python") || first_line.contains("python3") {
            return "python";
        }
        if first_line.contains("ruby") {
            return "ruby";
        }
        if first_line.contains("node") || first_line.contains("deno") {
            return "js";
        }
    }

    // PHP open tag (must be before HTML's `<` check)
    if first_line.starts_with("<?php") || first_line.starts_with("<?=") {
        return "php";
    }

    // HTML doctype or tag
    if first_line.starts_with("<!") || first_line.starts_with("<") {
        return "html";
    }

    // Rust-specific keywords (checked early for accuracy)
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

    // Java: class/interface/enum with optional visibility prefix or import/package
    if first_line.starts_with("public class")
        || first_line.starts_with("private class")
        || first_line.starts_with("public interface")
        || first_line.starts_with("public enum")
        || first_line.starts_with("import java.")
        || first_line.starts_with("import javax.")
        || first_line.starts_with("package ") && first_line.trim_end().ends_with(';')
        || first_line.starts_with("@Override")
        || first_line.starts_with("@SuppressWarnings")
        || first_line.starts_with("@Deprecated")
    {
        return "java";
    }

    // Ruby (before Python since `def` and `class` overlap)
    // Note: Ruby `class Name` has NO colon; Python `class Name:` has a colon — we check
    // for colon-less class to avoid false-positive Ruby detection on Python classes.
    if first_line.starts_with("def ") && !first_line.trim_end().ends_with(':')
        || first_line.starts_with("class ") && !first_line.trim_end().ends_with(':')
        || first_line.starts_with("module ")
        || first_line.starts_with("require ")
        || first_line.starts_with("attr_")
        || first_line.starts_with("puts ")
        || first_line.starts_with("unless ")
        || first_line.starts_with("end")
    {
        return "ruby";
    }

    // Go (before Java since `package` and `import` are also Java keywords)
    // `package` without semicolon → Go (`package main`);
    // `package com.example;` (with semicolon) falls through to Java.
    if first_line.starts_with("func ")
        || first_line.starts_with("package ") && !first_line.trim_end().ends_with(';')
        || first_line.starts_with("import \"")
        || first_line.starts_with("import (")
    {
        return "go";
    }

    // Python: `def`, `print(`, `from`, `class X:`, `import ` without braces
    if first_line.starts_with("def ") && first_line.trim_end().ends_with(':')
        || first_line.starts_with("print(")
        || first_line.starts_with("from ")
        || first_line.starts_with("class ") && first_line.trim_end().ends_with(':')
        || first_line.starts_with("import ") && !first_line.contains("{")
    {
        return "python";
    }

    // Bash/shell (before JS since `export` matches both)
    if first_line.starts_with("if ") && first_line.contains("then")
        || first_line.starts_with("for ") && first_line.contains("in ")
        || first_line.starts_with("while ") && first_line.contains("do ")
        || first_line.starts_with("case ") && first_line.contains("in")
        || first_line.starts_with("export ")
        || first_line.starts_with("source ")
        || first_line.starts_with("alias ")
        || first_line.starts_with("echo ")
        || first_line.starts_with("printf ")
    {
        return "bash";
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
    {
        return "rust";
    }

    // Fall through to C if struct/enum/union without braces (C style)
    if first_line.starts_with("struct ") || first_line.starts_with("enum ") || first_line.starts_with("union ") {
        return "c";
    }

    // PHP: namespace, use, function (after other languages checked)
    if first_line.starts_with("namespace ")
        || first_line.starts_with("function ") && first_line.trim_end().ends_with('(')
    {
        return "php";
    }

    // JS/TS module import/export
    if first_line.starts_with("import {")
        || first_line.starts_with("import *")
        || first_line.starts_with("import type")
        || first_line.starts_with("export ")
    {
        return "js";
    }

    // JS/TS general patterns (checked after more specific keywords)
    if first_line.starts_with("function ") {
        return "js";
    }

    // JSON: starts with { or [
    let trimmed = first_line.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return "json";
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

    #[test]
    fn highlight_python_wraps_in_colored_output() {
        let result = highlight_code("def hello():\n    print('hi')", "python");
        assert!(result.contains("def"));
        assert!(result.contains("hello"));
    }

    #[test]
    fn highlight_go_wraps_in_colored_output() {
        let result = highlight_code("func main() {\n\tfmt.Println(\"hello\")\n}", "go");
        assert!(result.contains("func"));
        assert!(result.contains("main"));
    }

    #[test]
    fn highlight_json_wraps_in_colored_output() {
        let result = highlight_code("{\"key\": \"value\", \"flag\": true}", "json");
        assert!(result.contains("key"));
        assert!(result.contains("value"));
    }

    #[test]
    fn detect_json_from_brace() {
        assert_eq!(detect_language_from_content("{\"key\": 1}"), "json");
    }

    #[test]
    fn detect_json_from_bracket() {
        assert_eq!(detect_language_from_content("[1, 2, 3]"), "json");
    }

    #[test]
    fn highlight_python_from_autodetect() {
        let input = "def foo():\n    pass";
        let lang = detect_language_from_content(input);
        assert_eq!(lang, "python");
        let result = highlight_code(input, lang);
        assert!(result.contains("def"));
    }

    #[test]
    fn highlight_go_from_autodetect() {
        let input = "func foo() {}";
        let lang = detect_language_from_content(input);
        assert_eq!(lang, "go");
        let result = highlight_code(input, lang);
        assert!(result.contains("func"));
    }

    #[test]
    fn highlight_c_wraps_in_colored_output() {
        let result = highlight_code("int main() {\n    return 0;\n}", "c");
        assert!(result.contains("int"));
        assert!(result.contains("return"));
    }

    #[test]
    fn highlight_java_wraps_in_colored_output() {
        let result = highlight_code(
            "public class Hello {\n    public static void main(String[] args) {}",
            "java",
        );
        assert!(result.contains("public"));
        assert!(result.contains("class"));
    }

    #[test]
    fn highlight_ruby_wraps_in_colored_output() {
        let result = highlight_code("def hello\n    puts 'hi'\nend", "ruby");
        assert!(result.contains("def"));
        assert!(result.contains("end"));
    }

    #[test]
    fn highlight_php_wraps_in_colored_output() {
        let result = highlight_code("<?php\necho 'hello';\n", "php");
        assert!(result.contains("echo"));
    }

    #[test]
    fn highlight_bash_wraps_in_colored_output() {
        let result = highlight_code("#!/bin/bash\necho \"hello\"", "bash");
        assert!(result.contains("echo"));
    }

    #[test]
    fn detect_java_from_class() {
        assert_eq!(detect_language_from_content("public class HelloWorld {"), "java");
    }

    #[test]
    fn detect_java_from_package() {
        assert_eq!(detect_language_from_content("package com.example;"), "java");
    }

    #[test]
    fn detect_ruby_from_def() {
        assert_eq!(detect_language_from_content("def hello"), "ruby");
    }

    #[test]
    fn detect_php_from_open_tag() {
        assert_eq!(detect_language_from_content("<?php"), "php");
    }

    #[test]
    fn detect_bash_from_shebang() {
        assert_eq!(detect_language_from_content("#!/bin/bash"), "bash");
    }

    #[test]
    fn detect_bash_from_export() {
        assert_eq!(detect_language_from_content("export PATH=$PATH:/usr/local/bin"), "bash");
    }
}
