#[path = "../src/intent.rs"]
mod intent;

#[path = "../src/parsing.rs"]
mod parsing;

#[allow(dead_code)]
mod _allow_dead_code_refs {
    use super::{intent, parsing};

    pub fn consume() {
        let _ = intent::intent_instruction;
        let _ = parsing::current_name_from_bold;
    }
}

#[test]
fn intent_identifies_code_action() {
    assert_eq!(
        intent::classify_intent("create a rust cli for notes"),
        intent::TaskIntent::CodeAction
    );
}

#[test]
fn intent_identifies_fast_answer() {
    assert_eq!(
        intent::classify_intent("explain rust ownership"),
        intent::TaskIntent::FastAnswer
    );
}

#[test]
fn parsing_extracts_named_file_header() {
    let text = "### index.html\n```html\n<h1>Hello</h1>\n```";
    let first = parsing::extract_code_block(text);
    assert_eq!(first, "<h1>Hello</h1>");
}

#[test]
fn parsing_strips_code_blocks_from_chat_text() {
    let text = "Answer:\n```js\nconst x = 1;\n```\nDone.";
    let stripped = parsing::strip_code_blocks(text);
    assert!(stripped.contains("Answer:"));
    assert!(stripped.contains("Done."));
    assert!(!stripped.contains("const x = 1"));
}
