//! User intent classification.
//! Analyzes user input to determine intent (FastAnswer, Planning, WebNeeded,
//! CodeAction) using keyword heuristics and phrase matching.
//!
//! The [`IntentClassifier`] trait allows different classification strategies
//! to be swapped in. The default [`HeuristicClassifier`] uses keyword matching.

use regex::RegexSet;
use std::sync::LazyLock;

/// Classified intent of a user's input message.
#[derive(Debug, PartialEq, Clone)]
pub enum TaskIntent {
    FastAnswer,
    Planning,
    WebNeeded,
    CodeAction,
}

/// Pluggable intent classifier trait.
/// Implementations analyze user input and return a [`TaskIntent`].
pub trait IntentClassifier: Send + Sync {
    fn classify(&self, input: &str) -> TaskIntent;
}

static VERB_PHRASES: &[&str] = &[
    "create a ",
    "create an ",
    "create the ",
    "create me a ",
    "create my ",
    "build a ",
    "build an ",
    "build the ",
    "build me a ",
    "build my ",
    "make a ",
    "make an ",
    "make the ",
    "make me a ",
    "make my ",
    "generate a ",
    "generate an ",
    "generate the ",
    "generate me a ",
    "scaffold a ",
    "scaffold an ",
    "scaffold the ",
    "code a ",
    "code an ",
    "code the ",
    "code me a ",
    "spin up a ",
    "spin up an ",
    "spin up the ",
];

static WRITE_OBJECTS: &[&str] = &[
    "write a file",
    "write a component",
    "write a function",
    "write a class",
    "write a module",
    "write a script",
    "write a test",
    "write a handler",
    "write a service",
    "write a hook",
    "write a config",
    "write a schema",
    "write a migration",
    "write a seed",
    "write a cli",
    "write a tool",
    "write an app",
    "write an api",
    "write an endpoint",
];

static SUFFIX_PHRASES: &[&str] = &[
    "a file",
    "an app",
    "a component",
    "a project",
    "a website",
    "a script",
    "a page",
    "a module",
    "a class",
    "a function",
    "a service",
    "a handler",
    "a hook",
    "a config",
    "a schema",
    "a migration",
    "a seed",
    "a test",
    "a cli",
    "a tool",
    "a layout",
    "a route",
    "an endpoint",
    "an api",
];

static VERB_ROOTS: &[&str] = &[
    "create", "build", "generate", "scaffold", "write", "code", "make", "spin up",
];

// Pre-compiled RegexSet for efficient concurrent matching
static QUESTION_RE: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new([
        "^how to",
        "^how do i",
        "^how do you",
        "^how can i",
        "^how would you",
        "^how should i",
        "^what is the best way to",
        "^what's the best way to",
        "^explain how to",
        "^tell me how to",
        "^describe how to",
        "^show me how to",
        "^can you explain how to",
        "^can you show me how to",
        "^why should i",
        "^when should i",
        "^where should i",
        "^what is",
        "^what are",
        "^what does",
        "^how does",
        "^why is",
        "^why are",
        "^why does",
        "^tell me about",
        "^describe",
        "^define",
    ])
    .expect("invalid question regex")
});

// Web intent patterns
static WEB_RE: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new([
        "search the web",
        "search online",
        "latest version",
        "latest release",
        "npm package",
        "pip install",
        "api docs",
        "api documentation",
        "stripe api",
        "github repo",
        "browse http",
        "stack overflow",
        "look up the",
        "on the internet",
    ])
    .expect("invalid web regex")
});

// Planning intent patterns
static PLANNING_RE: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new([
        "suggest an approach",
        "how should i",
        "what's the best way",
        "what is the best way",
        "suggest a strategy",
        "design a system",
        "what are the trade",
        "should i use",
        "would you recommend",
        "is it better to",
    ])
    .expect("invalid planning regex")
});

// Fix/refactor intent patterns (word boundaries ensure prefix-like matching)
static FIX_RE: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new([
        "^fix the ",
        "^fix my ",
        "^fix this ",
        "^refactor the ",
        "^refactor my ",
        "^rename the ",
        "^delete the ",
        "^remove the ",
        "^optimize the ",
        "^update the ",
        "^update my ",
    ])
    .expect("invalid fix regex")
});

// Verb phrase space-prefixed patterns (for contains matching)
static VERB_SPACE_RE: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new(
        VERB_PHRASES
            .iter()
            .map(|p| format!(" {}", p.trim_end()))
            .collect::<Vec<_>>(),
    )
    .expect("invalid verb-space regex")
});

static WRITE_OBJECTS_RE: LazyLock<RegexSet> =
    LazyLock::new(|| RegexSet::new(WRITE_OBJECTS).expect("invalid write-objects regex"));

static VERB_PHRASES_RE: LazyLock<RegexSet> =
    LazyLock::new(|| RegexSet::new(VERB_PHRASES).expect("invalid verb-phrases regex"));

/// Heuristic keyword-based intent classifier.
pub struct HeuristicClassifier;

impl IntentClassifier for HeuristicClassifier {
    fn classify(&self, input: &str) -> TaskIntent {
        classify_intent_heuristic(input)
    }
}

/// Default global intent classifier instance.
static CLASSIFIER: LazyLock<HeuristicClassifier> = LazyLock::new(|| HeuristicClassifier);

/// Classifies user input into a [`TaskIntent`] using keyword heuristics.
/// Delegates to the global default [`IntentClassifier`].
pub fn classify_intent(input: &str) -> TaskIntent {
    CLASSIFIER.classify(input)
}

// ── Heuristic implementation ────────────────────────────────────────────────

fn detect_web_intent(lower: &str) -> bool {
    WEB_RE.is_match(lower)
}

fn detect_planning_intent(lower: &str) -> bool {
    PLANNING_RE.is_match(lower)
        || (lower.contains("how to")
            && (lower.contains("implement")
                || lower.contains("architect")
                || lower.contains("design")
                || lower.contains("structure")))
}

fn detect_fix_intent(lower: &str) -> bool {
    FIX_RE.is_match(lower)
}

fn classify_intent_heuristic(input: &str) -> TaskIntent {
    let lower = input.to_lowercase();

    if detect_web_intent(&lower) {
        return TaskIntent::WebNeeded;
    }

    let is_question = has_question_prefix_lower(&lower);

    if detect_planning_intent(&lower) && !has_creation_intent_lower(&lower) {
        return TaskIntent::Planning;
    }

    if has_creation_intent_lower(&lower) {
        return TaskIntent::CodeAction;
    }

    if detect_fix_intent(&lower) && !is_question {
        return TaskIntent::CodeAction;
    }

    TaskIntent::FastAnswer
}

/// Returns a system prompt suffix based on the classified intent.
pub fn intent_instruction(intent: &TaskIntent) -> &'static str {
    match intent {
        TaskIntent::FastAnswer => "\n\n[ANSWER CONCISELY — no code generation, no file format. Just a clear text response. If uncertain whether user wants code, ask first.]",
        TaskIntent::Planning => "\n\n[PLAN FIRST — do NOT generate code. Give alternatives, trade-offs, and a recommendation. The user will tell you when to start coding.]",
        TaskIntent::WebNeeded => "\n\n[WEB SEARCH NEEDED — tell the user to run /search <query> to get current info before you can answer accurately.]",
        TaskIntent::CodeAction => "\n\n[USER WANTS CODE — first summarize what you'll create, then output files using the multi-file format.]",
    }
}

fn has_creation_intent_lower(lower: &str) -> bool {
    if !has_question_prefix_lower(lower) {
        // Fast path: check starts_with patterns via RegexSet
        if VERB_PHRASES_RE.is_match(lower) || VERB_SPACE_RE.is_match(lower) {
            return true;
        }
        // Check write objects
        if WRITE_OBJECTS_RE.is_match(lower) {
            return true;
        }
        // Check verb+suffix combinations: avoid O(160) pre-generated strings,
        // instead check if input has any verb root AND any suffix phrase
        let has_verb = VERB_ROOTS.iter().any(|v| lower.contains(v));
        let has_suffix = SUFFIX_PHRASES.iter().any(|s| lower.contains(s));
        if has_verb && has_suffix {
            return true;
        }
    }
    false
}

/// Checks whether input has creation/build/generation intent.
pub fn has_creation_intent(input: &str) -> bool {
    has_creation_intent_lower(&input.to_lowercase())
}

fn has_question_prefix_lower(lower: &str) -> bool {
    QUESTION_RE.is_match(lower)
}

fn _is_question_about_lower(lower: &str, _action_phrase: &str) -> bool {
    has_question_prefix_lower(lower)
}

/// Detects whether input contains file path references (extensions, `/`, etc.).
pub fn has_file_path(input: &str) -> bool {
    let lower = input.to_lowercase();
    lower.contains(".html")
        || lower.contains(".css")
        || lower.contains(".js")
        || lower.contains(".ts")
        || lower.contains(".py")
        || lower.contains(".rs")
        || lower.contains(".json")
        || lower.contains(".toml")
        || lower.contains(".yaml")
        || lower.contains(".yml")
        || lower.contains(".md")
        || lower.contains(".txt")
        || lower.contains(".go")
        || lower.contains(".dart")
        || lower.contains(".sh")
        || (lower.contains("/") && !lower.contains("://"))
        || lower.contains("into ./")
        || lower.contains("into /")
        || lower.contains("save to ")
        || lower.contains("save at ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_code_action_for_fix() {
        assert_eq!(classify_intent("fix this function"), TaskIntent::CodeAction);
    }

    #[test]
    fn classify_planning_for_strategy_prompt() {
        assert_eq!(
            classify_intent("what's the best way to structure this backend"),
            TaskIntent::Planning
        );
    }

    #[test]
    fn classify_web_needed_for_latest_release() {
        assert_eq!(
            classify_intent("what is the latest release of react"),
            TaskIntent::WebNeeded
        );
    }

    #[test]
    fn creation_intent_ignores_question_form() {
        assert!(!has_creation_intent("how do i create a file"));
        assert!(has_creation_intent("create a file called app.js"));
    }

    #[test]
    fn detects_file_paths_with_extensions() {
        assert!(has_file_path("update src/main.rs"));
        assert!(!has_file_path("explain how rust ownership works"));
    }

    #[test]
    fn classify_code_action_for_create_project() {
        assert_eq!(classify_intent("create a rust cli for notes"), TaskIntent::CodeAction);
    }

    #[test]
    fn classify_fast_answer_for_explain() {
        assert_eq!(classify_intent("explain rust ownership"), TaskIntent::FastAnswer);
    }

    #[test]
    fn has_file_path_ignores_urls_in_middle() {
        assert!(!has_file_path("check http://example.com/api"));
        assert!(!has_file_path("see https://example.com/path/file"));
    }
}
