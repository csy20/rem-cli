use std::sync::LazyLock;

#[derive(Debug, PartialEq, Clone)]
pub enum TaskIntent {
    FastAnswer,
    Planning,
    WebNeeded,
    CodeAction,
}

static VERB_PHRASES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
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
    ]
});

static WRITE_OBJECTS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
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
    ]
});

static SUFFIX_PHRASES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
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
    ]
});

static VERB_ROOTS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "create", "build", "generate", "scaffold", "write", "code", "make", "spin up",
    ]
});

static QUESTION_PREFIXES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "how to",
        "how do i",
        "how do you",
        "how can i",
        "how would you",
        "how should i",
        "what is the best way to",
        "what's the best way to",
        "explain how to",
        "tell me how to",
        "describe how to",
        "show me how to",
        "can you explain how to",
        "can you show me how to",
        "why should i",
        "when should i",
        "where should i",
        "what is",
        "what are",
        "what does",
        "how does",
        "why is",
        "why are",
        "why does",
        "tell me about",
        "describe",
        "define",
    ]
});

static PHRASE_COMBINATIONS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut out = Vec::new();
    for verb in VERB_ROOTS.iter() {
        for suffix in SUFFIX_PHRASES.iter() {
            // Leak the Box to get &'static str
            let s: &'static str = Box::leak(format!("{} {}", verb, suffix).into_boxed_str());
            out.push(s);
        }
    }
    out
});

static VERB_PHRASES_SPACE: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    VERB_PHRASES
        .iter()
        .map(|p| {
            let leaked: &'static mut str = Box::leak(format!(" {}", p).into_boxed_str());
            let static_ref: &'static str = leaked;
            static_ref
        })
        .collect()
});

static PHRASE_COMBINATIONS_SPACE: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    PHRASE_COMBINATIONS
        .iter()
        .map(|p| {
            let leaked: &'static mut str = Box::leak(format!(" {}", p).into_boxed_str());
            let static_ref: &'static str = leaked;
            static_ref
        })
        .collect()
});

pub fn classify_intent(input: &str) -> TaskIntent {
    let lower = input.to_lowercase();

    let web_explicit = lower.contains("search the web")
        || lower.contains("search online")
        || lower.contains("latest version")
        || lower.contains("latest release")
        || lower.contains("npm package")
        || lower.contains("pip install")
        || lower.contains("api docs")
        || lower.contains("api documentation")
        || lower.contains("stripe api")
        || lower.contains("github repo")
        || lower.contains("browse http")
        || lower.contains("stack overflow")
        || lower.contains("look up the")
        || lower.contains("on the internet");

    if web_explicit {
        return TaskIntent::WebNeeded;
    }

    let is_question = lower.starts_with("what ")
        || lower.starts_with("how ")
        || lower.starts_with("why ")
        || lower.starts_with("when ")
        || lower.starts_with("where ")
        || lower.starts_with("who ")
        || lower.starts_with("can you explain")
        || lower.starts_with("explain ")
        || lower.starts_with("describe ")
        || lower.starts_with("tell me ")
        || lower.starts_with("show me ");

    let plan_indicators = lower.contains("how would you")
        || lower.contains("how should i")
        || lower.contains("what's the best way")
        || lower.contains("what is the best way")
        || lower.contains("suggest an approach")
        || lower.contains("suggest a strategy")
        || lower.contains("design a system")
        || lower.contains("what are the trade")
        || lower.contains("should i use")
        || lower.contains("would you recommend")
        || lower.contains("is it better to")
        || (lower.contains("how to") && lower.contains("implement"))
        || (lower.contains("how to") && lower.contains("architect"))
        || (lower.contains("how to") && lower.contains("design"))
        || (lower.contains("how to") && lower.contains("structure"));

    if plan_indicators && !has_creation_intent_lower(&lower) {
        return TaskIntent::Planning;
    }

    let has_create = has_creation_intent_lower(&lower);

    if is_question && has_create {
        return TaskIntent::FastAnswer;
    }

    if has_create {
        return TaskIntent::CodeAction;
    }

    let fix_indicators = lower.starts_with("fix the ")
        || lower.starts_with("fix my ")
        || lower.starts_with("fix this ")
        || lower.starts_with("refactor the ")
        || lower.starts_with("refactor my ")
        || lower.starts_with("rename the ")
        || lower.starts_with("delete the ")
        || lower.starts_with("remove the ")
        || lower.starts_with("optimize the ")
        || lower.starts_with("update the ")
        || lower.starts_with("update my ");

    if fix_indicators && !is_question {
        return TaskIntent::CodeAction;
    }

    TaskIntent::FastAnswer
}

pub fn intent_instruction(intent: &TaskIntent) -> &'static str {
    match intent {
        TaskIntent::FastAnswer => "\n\n[ANSWER CONCISELY — no code generation, no file format. Just a clear text response. If uncertain whether user wants code, ask first.]",
        TaskIntent::Planning => "\n\n[PLAN FIRST — do NOT generate code. Give alternatives, trade-offs, and a recommendation. The user will tell you when to start coding.]",
        TaskIntent::WebNeeded => "\n\n[WEB SEARCH NEEDED — tell the user to run /search <query> to get current info before you can answer accurately.]",
        TaskIntent::CodeAction => "\n\n[USER WANTS CODE — first summarize what you'll create, then output files using the multi-file format.]",
    }
}

fn has_creation_intent_lower(lower: &str) -> bool {
    if (VERB_PHRASES.iter().any(|v| lower.starts_with(v))
        || VERB_PHRASES_SPACE.iter().any(|v| lower.contains(v)))
        && !has_question_prefix_lower(lower)
    {
        return true;
    }

    if WRITE_OBJECTS.iter().any(|w| lower.contains(w)) && !has_question_prefix_lower(lower) {
        return true;
    }

    for (combined, combined_space) in PHRASE_COMBINATIONS
        .iter()
        .zip(PHRASE_COMBINATIONS_SPACE.iter())
    {
        if (lower.starts_with(combined) || lower.contains(combined_space))
            && !is_question_about_lower(lower, combined)
        {
            return true;
        }
    }

    false
}

pub fn has_creation_intent(input: &str) -> bool {
    has_creation_intent_lower(&input.to_lowercase())
}

fn has_question_prefix_lower(lower: &str) -> bool {
    QUESTION_PREFIXES.iter().any(|p| lower.starts_with(p))
}

fn is_question_about_lower(lower: &str, action_phrase: &str) -> bool {
    if !lower.contains(action_phrase) {
        return false;
    }
    has_question_prefix_lower(lower)
}

#[allow(dead_code)]
pub(crate) fn is_question_about(input: &str, action_phrase: &str) -> bool {
    is_question_about_lower(&input.to_lowercase(), action_phrase)
}

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
        || (lower.contains("/") && !lower.starts_with("http"))
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
}
