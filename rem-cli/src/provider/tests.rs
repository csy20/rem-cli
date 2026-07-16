use super::*;
use crate::ModelReply;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_api_url_without_api_suffix() {
    let url = ollama_api_url("http://localhost:11434", "tags");
    assert_eq!(url, "http://localhost:11434/api/tags");
}

#[tokio::test]
async fn test_api_url_with_api_suffix() {
    let url = ollama_api_url("http://localhost:11434/api", "tags");
    assert_eq!(url, "http://localhost:11434/api/tags");
}

#[tokio::test]
async fn test_api_url_trailing_slash() {
    let url = ollama_api_url("http://localhost:11434/", "generate");
    assert_eq!(url, "http://localhost:11434/api/generate");
}

#[tokio::test]
async fn test_provider_kind_from_str() {
    assert_eq!(ProviderKind::from_str("ollama"), ProviderKind::Ollama);
    assert_eq!(ProviderKind::from_str("openai"), ProviderKind::OpenAI);
    assert_eq!(ProviderKind::from_str("gemini"), ProviderKind::Gemini);
    assert_eq!(ProviderKind::from_str("google"), ProviderKind::Gemini);
    assert_eq!(ProviderKind::from_str("anthropic"), ProviderKind::Anthropic);
    assert_eq!(ProviderKind::from_str("claude"), ProviderKind::Anthropic);
    assert_eq!(ProviderKind::from_str("azure"), ProviderKind::Azure);
    assert_eq!(ProviderKind::from_str("deepseek"), ProviderKind::DeepSeek);
    assert_eq!(ProviderKind::from_str("github"), ProviderKind::GitHub);
    assert_eq!(ProviderKind::from_str("githubmodels"), ProviderKind::GitHub);
    assert_eq!(ProviderKind::from_str("xai"), ProviderKind::XAI);
    assert_eq!(ProviderKind::from_str("grok"), ProviderKind::XAI);
    assert_eq!(ProviderKind::from_str("openrouter"), ProviderKind::OpenRouter);
    assert_eq!(ProviderKind::from_str("unknown"), ProviderKind::Ollama);
}

#[tokio::test]
async fn test_provider_kind_as_str() {
    assert_eq!(ProviderKind::Ollama.as_str(), "ollama");
    assert_eq!(ProviderKind::OpenAI.as_str(), "openai");
    assert_eq!(ProviderKind::Gemini.as_str(), "gemini");
    assert_eq!(ProviderKind::Anthropic.as_str(), "anthropic");
    assert_eq!(ProviderKind::Azure.as_str(), "azure");
    assert_eq!(ProviderKind::OpenRouter.as_str(), "openrouter");
    assert_eq!(ProviderKind::DeepSeek.as_str(), "deepseek");
    assert_eq!(ProviderKind::GitHub.as_str(), "github");
    assert_eq!(ProviderKind::XAI.as_str(), "xai");
}

// ── Ollama ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_ollama_list_models() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "models": [
                {"name": "rem-coder:latest"},
                {"name": "llama3:8b"}
            ]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Ollama,
        mock_server.uri(),
        "rem-coder:latest".to_string(),
        30,
        String::new(),
        None,
        4096,
    );

    let models = provider.list_models().await.unwrap();
    assert_eq!(models, vec!["rem-coder:latest", "llama3:8b"]);
}

#[tokio::test]
async fn test_ollama_complete_json() {
    let mock_server = MockServer::start().await;
    let response = ModelReply {
        explanation: "test".to_string(),
        code: "fn main() {}".to_string(),
        files: vec![],
        commands: vec![],
        checks: vec![],
        caution: String::new(),
    };
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "response": serde_json::to_string(&response).unwrap()
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Ollama,
        mock_server.uri(),
        "rem-coder:latest".to_string(),
        30,
        "You are REM.".to_string(),
        None,
        4096,
    );

    let reply = provider.complete_json("write hello").await.unwrap();
    assert_eq!(reply.explanation, "test");
    assert_eq!(reply.code, "fn main() {}");
}

#[tokio::test]
async fn test_ollama_complete_json_fallback() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "response": "this is not valid json"
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Ollama,
        mock_server.uri(),
        "rem-coder:latest".to_string(),
        30,
        String::new(),
        None,
        4096,
    );

    let reply = provider.complete_json("hello").await.unwrap();
    assert_eq!(reply.explanation, "this is not valid json");
}

#[tokio::test]
async fn test_ollama_model_not_found() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": "model 'unknown' not found"
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Ollama,
        mock_server.uri(),
        "unknown".to_string(),
        30,
        String::new(),
        None,
        4096,
    );

    let err = provider.complete_json("hello").await.unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.contains("not found"));
    assert!(msg.contains("ollama pull"));
}

#[tokio::test]
async fn test_ollama_chat_stream() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "{\"message\":{\"content\":\"Hello\"},\"done\":false}\n{\"message\":{\"content\":\" World\"},\"done\":false}\n{\"message\":{\"content\":\"\"},\"done\":true}\n"
        ))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Ollama,
        mock_server.uri(),
        "rem-coder:latest".to_string(),
        30,
        String::new(),
        None,
        4096,
    );

    let result = provider.complete_chat_stream("hi", "", "").await.unwrap();
    assert_eq!(result, "Hello World");
}

// ── OpenAI ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_openai_list_models() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "gpt-4"}, {"id": "gpt-3.5-turbo"}]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::OpenAI,
        mock_server.uri(),
        "gpt-4".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let models = provider.list_models().await.unwrap();
    assert_eq!(models, vec!["gpt-4", "gpt-3.5-turbo"]);
}

#[tokio::test]
async fn test_openai_complete_json() {
    let mock_server = MockServer::start().await;
    let response_str =
        r#"{"explanation":"test","code":"print('hello')","files":[],"commands":[],"checks":[],"caution":""}"#;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{"message": {"content": response_str}}]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::OpenAI,
        mock_server.uri(),
        "gpt-4".to_string(),
        30,
        "Be helpful.".to_string(),
        Some("test-key".to_string()),
        4096,
    );

    let reply = provider.complete_json("write code").await.unwrap();
    assert_eq!(reply.explanation, "test");
    assert_eq!(reply.code, "print('hello')");
}

#[tokio::test]
async fn test_openai_chat_stream() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\ndata: {\"choices\":[{\"delta\":{\"content\":\" World\"}}]}\ndata: [DONE]\n"
        ))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::OpenAI,
        mock_server.uri(),
        "gpt-4".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let result = provider.complete_chat_stream("hi", "", "").await.unwrap();
    assert_eq!(result, "Hello World");
}

#[tokio::test]
async fn test_openai_api_error() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": "invalid_api_key"
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::OpenAI,
        mock_server.uri(),
        "gpt-4".to_string(),
        30,
        String::new(),
        Some("bad-key".to_string()),
        4096,
    );

    let err = provider.complete_json("hello").await.unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.contains("OpenAI"));
    assert!(msg.contains("401"));
}

// ── Gemini ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_gemini_list_models() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "models": [
                {"name": "models/gemini-2.0-flash", "display_name": "Gemini 2.0 Flash"},
                {"name": "models/gemini-pro", "display_name": "Gemini Pro"}
            ]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Gemini,
        mock_server.uri(),
        "gemini-2.0-flash".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let models = provider.list_models().await.unwrap();
    assert!(models.contains(&"gemini-2.0-flash".to_string()));
}

#[tokio::test]
async fn test_gemini_complete_json() {
    let mock_server = MockServer::start().await;
    let response_str = r#"{"explanation":"test","code":"","files":[],"commands":[],"checks":[],"caution":""}"#;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": response_str}]
                }
            }]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Gemini,
        mock_server.uri(),
        "gemini-2.0-flash".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let reply = provider.complete_json("hello").await.unwrap();
    assert_eq!(reply.explanation, "test");
}

#[tokio::test]
async fn test_gemini_chat_stream() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\ndata: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" World\"}]}}]}\n"
        ))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Gemini,
        mock_server.uri(),
        "gemini-2.0-flash".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let result = provider.complete_chat_stream("hi", "", "").await.unwrap();
    assert_eq!(result, "Hello World");
}

// ── Anthropic ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_anthropic_list_models() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {"type": "model", "id": "claude-sonnet-4-20250514", "display_name": "Claude Sonnet 4"},
                {"type": "model", "id": "claude-haiku-3-5", "display_name": "Claude Haiku 3.5"}
            ]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Anthropic,
        mock_server.uri(),
        "claude-sonnet-4-20250514".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let models = provider.list_models().await.unwrap();
    assert!(models.contains(&"claude-sonnet-4-20250514".to_string()));
}

#[tokio::test]
async fn test_anthropic_complete_json() {
    let mock_server = MockServer::start().await;
    let response_str = r#"{"explanation":"test","code":"","files":[],"commands":[],"checks":[],"caution":""}"#;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "content": [{"text": response_str}]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Anthropic,
        mock_server.uri(),
        "claude-sonnet-4-20250514".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let reply = provider.complete_json("hello").await.unwrap();
    assert_eq!(reply.explanation, "test");
}

#[tokio::test]
async fn test_anthropic_chat_stream() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hello\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\" World\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n"
        ))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Anthropic,
        mock_server.uri(),
        "claude-sonnet-4-20250514".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let result = provider.complete_chat_stream("hi", "", "").await.unwrap();
    assert_eq!(result, "Hello World");
}

#[tokio::test]
async fn test_anthropic_api_error() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": {"message": "Invalid request"}
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Anthropic,
        mock_server.uri(),
        "claude-sonnet-4-20250514".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let err = provider.complete_json("hello").await.unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.contains("Anthropic"));
}

#[tokio::test]
async fn test_gemini_api_error() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": {"message": "API key not valid"}
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Gemini,
        mock_server.uri(),
        "gemini-2.0-flash".to_string(),
        30,
        String::new(),
        Some("bad-key".to_string()),
        4096,
    );

    let err = provider.complete_json("hello").await.unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.contains("Gemini"));
}

// ── Azure ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_azure_list_models() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "gpt-4"}, {"id": "gpt-35-turbo"}]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Azure,
        mock_server.uri(),
        "gpt-4".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let models = provider.list_models().await.unwrap();
    assert_eq!(models, vec!["gpt-4"]);
}

#[tokio::test]
async fn test_azure_complete_json() {
    let mock_server = MockServer::start().await;
    let response_str =
        r#"{"explanation":"azure test","code":"print('azure')","files":[],"commands":[],"checks":[],"caution":""}"#;
    Mock::given(method("POST"))
        .and(path("/openai/deployments/gpt-4/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{"message": {"content": response_str}}]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Azure,
        mock_server.uri(),
        "gpt-4".to_string(),
        30,
        "Be helpful.".to_string(),
        Some("test-key".to_string()),
        4096,
    );

    let reply = provider.complete_json("write code").await.unwrap();
    assert_eq!(reply.explanation, "azure test");
    assert_eq!(reply.code, "print('azure')");
}

#[tokio::test]
async fn test_azure_chat_stream() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/openai/deployments/gpt-4/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\ndata: {\"choices\":[{\"delta\":{\"content\":\" from Azure\"}}]}\ndata: [DONE]\n"
        ))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Azure,
        mock_server.uri(),
        "gpt-4".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let result = provider.complete_chat_stream("hi", "", "").await.unwrap();
    assert_eq!(result, "Hello from Azure");
}

#[tokio::test]
async fn test_azure_api_error() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/openai/deployments/gpt-4/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": "invalid_api_key"
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::Azure,
        mock_server.uri(),
        "gpt-4".to_string(),
        30,
        String::new(),
        Some("bad-key".to_string()),
        4096,
    );

    let err = provider.complete_json("hello").await.unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.contains("Azure"));
}

// ── OpenRouter ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_openrouter_list_models() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "gpt-4"}, {"id": "claude-sonnet-4"}]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::OpenRouter,
        mock_server.uri(),
        "gpt-4".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let models = provider.list_models().await.unwrap();
    assert_eq!(models, vec!["gpt-4", "claude-sonnet-4"]);
}

#[tokio::test]
async fn test_openrouter_complete_json() {
    let mock_server = MockServer::start().await;
    let response_str =
        r#"{"explanation":"or test","code":"print('or')","files":[],"commands":[],"checks":[],"caution":""}"#;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{"message": {"content": response_str}}]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::OpenRouter,
        mock_server.uri(),
        "gpt-4".to_string(),
        30,
        "Be helpful.".to_string(),
        Some("test-key".to_string()),
        4096,
    );

    let reply = provider.complete_json("write code").await.unwrap();
    assert_eq!(reply.explanation, "or test");
    assert_eq!(reply.code, "print('or')");
}

#[tokio::test]
async fn test_openrouter_chat_stream() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\ndata: {\"choices\":[{\"delta\":{\"content\":\" from OpenRouter\"}}]}\ndata: [DONE]\n"
        ))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::OpenRouter,
        mock_server.uri(),
        "gpt-4".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let result = provider.complete_chat_stream("hi", "", "").await.unwrap();
    assert_eq!(result, "Hello from OpenRouter");
}

#[test]
fn is_transient_detects_timeout_string() {
    let err = anyhow::anyhow!("request timed out");
    assert!(Provider::is_transient_error(&err));
}

#[test]
fn is_transient_detects_connection_refused() {
    let err = anyhow::anyhow!("connection refused: endpoint");
    assert!(Provider::is_transient_error(&err));
}

#[test]
fn is_transient_handles_unexpected_format() {
    let err = anyhow::anyhow!("unexpected response format");
    assert!(!Provider::is_transient_error(&err));
}

#[test]
fn is_transient_rejects_authentication_errors() {
    let err = anyhow::anyhow!("invalid API key");
    assert!(!Provider::is_transient_error(&err));
}

#[test]
fn is_transient_rejects_400() {
    let err = anyhow::anyhow!("HTTP 400 Bad Request");
    assert!(!Provider::is_transient_error(&err));
}

// ── Shared utility tests ──────────────────────────────────────────────

#[test]
fn parse_history_turns_empty() {
    let turns = parse_history_turns("");
    assert!(turns.is_empty(), "empty history should yield no turns");
}

#[test]
fn parse_history_turns_single_pair() {
    let input = "User: hello\n<<<REM:BOUNDARY>>>\nHi there!";
    let turns = parse_history_turns(input);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].0, "hello");
    assert_eq!(turns[0].1, "Hi there!");
}

#[test]
fn parse_history_turns_multiple_pairs() {
    let input = "User: first\n<<<REM:BOUNDARY>>>\nanswer1\n\nUser: second\n<<<REM:BOUNDARY>>>\nanswer2";
    let turns = parse_history_turns(input);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].0, "first");
    assert_eq!(turns[0].1, "answer1");
    assert_eq!(turns[1].0, "second");
    assert_eq!(turns[1].1, "answer2");
}

#[test]
fn parse_history_turns_no_assistant() {
    let input = "User: only user message\n<<<REM:BOUNDARY>>>\n";
    let turns = parse_history_turns(input);
    assert_eq!(turns.len(), 1);
    // The user part strips "User: " prefix
    assert!(
        turns[0].0.contains("only user message"),
        "user content should contain the message"
    );
    assert!(
        turns[0].1.is_empty() || turns[0].1.trim().is_empty(),
        "assistant content should be empty or whitespace"
    );
}

#[test]
fn parse_history_turns_with_newline_escaping() {
    let input = "User: line1\\nline2\n<<<REM:BOUNDARY>>>\nresponse";
    let turns = parse_history_turns(input);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].0, "line1\nline2", "escaped newlines should be unescaped");
}

#[test]
fn build_messages_from_history_empty() {
    let messages = build_messages_from_history("", "user prompt", Some("system prompt"));
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "system prompt");
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages[1]["content"], "user prompt");
}

#[test]
fn build_messages_from_history_with_turns() {
    let messages = build_messages_from_history("User: prev user\n<<<REM:BOUNDARY>>>\nprev asst", "new prompt", None);
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"], "prev user");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"], "prev asst");
    assert_eq!(messages[2]["role"], "user");
    assert_eq!(messages[2]["content"], "new prompt");
}

#[test]
fn parse_history_turns_user_content_containing_delimiter() {
    // User message containing "REM:" should not confuse parsing
    let input = "User: I said REM: remember this\n<<<REM:BOUNDARY>>>\nGot it!";
    let turns = parse_history_turns(input);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].0, "I said REM: remember this");
    assert_eq!(turns[0].1, "Got it!");
}

#[test]
fn parse_history_turns_empty_history() {
    assert!(parse_history_turns("").is_empty());
    assert!(parse_history_turns(" ").is_empty());
    assert!(parse_history_turns("\n\n").is_empty());
}

#[test]
fn parse_history_turns_unicode_preservation() {
    let input = "User: café résumé 日本語\n<<<REM:BOUNDARY>>>\nпривет world 🌍";
    let turns = parse_history_turns(input);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].0, "café résumé 日本語");
    assert_eq!(turns[0].1, "привет world 🌍");
}

#[test]
fn parse_history_turns_multi_turn_empty_assistant() {
    let input = "User: first\n<<<REM:BOUNDARY>>>\n\n\nUser: second\n<<<REM:BOUNDARY>>>\nanswer2";
    let turns = parse_history_turns(input);
    assert_eq!(turns.len(), 2);
    assert!(turns[0].1.is_empty() || turns[0].1.trim().is_empty());
    assert_eq!(turns[1].1, "answer2");
}

#[test]
fn is_transient_error_429() {
    let err = anyhow::anyhow!("HTTP 429 Too Many Requests");
    assert!(Provider::is_transient_error(&err), "429 should be transient");
}

#[test]
fn is_transient_error_500() {
    let err = anyhow::anyhow!("HTTP 500 Internal Server Error");
    assert!(Provider::is_transient_error(&err), "5xx should be transient");
}

#[test]
fn openai_chat_url_basic() {
    let url = openai_chat_url("https://api.openai.com/v1", ProviderKind::OpenAI, "gpt-4");
    assert_eq!(url, "https://api.openai.com/v1/chat/completions");
}

#[test]
fn openai_chat_url_azure() {
    let url = openai_chat_url("https://my-azure.openai.azure.com", ProviderKind::Azure, "gpt-4");
    assert!(url.contains("gpt-4"));
    assert!(url.contains("api-version=2024-02-15-preview"));
}

#[test]
fn openai_models_url_basic() {
    let url = openai_models_url("https://api.openai.com/v1");
    assert_eq!(url, "https://api.openai.com/v1/models");
}

#[test]
fn default_base_url_ollama() {
    let url = default_base_url(ProviderKind::Ollama);
    assert_eq!(url, "http://localhost:11434");
}

#[test]
fn default_base_url_openai() {
    let url = default_base_url(ProviderKind::OpenAI);
    assert_eq!(url, "https://api.openai.com/v1");
}

#[test]
fn default_base_url_unknown_fallback() {
    let url = default_base_url(ProviderKind::Bedrock);
    assert_eq!(url, "", "Bedrock has no default base URL");
}

#[test]
fn api_key_env_var_lookup() {
    assert_eq!(api_key_env_var(ProviderKind::OpenAI), Some("OPENAI_API_KEY"));
    assert_eq!(api_key_env_var(ProviderKind::Anthropic), Some("ANTHROPIC_API_KEY"));
    assert_eq!(
        api_key_env_var(ProviderKind::Ollama),
        None,
        "Ollama has no API key env var"
    );
}

#[test]
fn provider_label_format() {
    use crate::config::build_provider;
    let cfg = crate::cli::AppConfig::default();
    if let Ok(provider) = build_provider(&cfg, String::new()) {
        let label = provider.provider_label();
        assert!(label.contains("ollama"), "default provider label should mention ollama");
    }
}

#[test]
fn parse_json_fallback_valid() {
    let json = r#"{"explanation":"valid","code":"fn main(){}","files":[],"commands":[],"checks":[],"caution":""}"#;
    let result = parse_json_fallback(json).unwrap();
    assert_eq!(result.explanation, "valid");
}

#[test]
fn parse_json_fallback_invalid() {
    let result = parse_json_fallback("not json at all").unwrap();
    assert_eq!(result.explanation, "not json at all", "fallback should use raw text");
}

#[test]
fn ollama_api_url_trailing_slash_base() {
    let url = ollama_api_url("http://localhost:11434/", "tags");
    assert_eq!(url, "http://localhost:11434/api/tags");
}

// ── DeepSeek ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_deepseek_list_models() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "deepseek-chat"}, {"id": "deepseek-reasoner"}]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::DeepSeek,
        mock_server.uri(),
        "deepseek-chat".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let models = provider.list_models().await.unwrap();
    assert_eq!(models, vec!["deepseek-chat", "deepseek-reasoner"]);
}

#[tokio::test]
async fn test_deepseek_complete_json() {
    let mock_server = MockServer::start().await;
    let response_str = r#"{"explanation":"deepseek test","code":"print('deepseek')","files":[],"commands":[],"checks":[],"caution":""}"#;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{"message": {"content": response_str}}]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::DeepSeek,
        mock_server.uri(),
        "deepseek-chat".to_string(),
        30,
        "Be helpful.".to_string(),
        Some("test-key".to_string()),
        4096,
    );

    let reply = provider.complete_json("write code").await.unwrap();
    assert_eq!(reply.explanation, "deepseek test");
    assert_eq!(reply.code, "print('deepseek')");
}

#[tokio::test]
async fn test_deepseek_chat_stream() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\ndata: {\"choices\":[{\"delta\":{\"content\":\" from DeepSeek\"}}]}\ndata: [DONE]\n"
        ))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::DeepSeek,
        mock_server.uri(),
        "deepseek-chat".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let result = provider.complete_chat_stream("hi", "", "").await.unwrap();
    assert_eq!(result, "Hello from DeepSeek");
}

// ── GitHub Models ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_github_list_models() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "gpt-4o"}, {"id": "gpt-4o-mini"}]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::GitHub,
        mock_server.uri(),
        "gpt-4o".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let models = provider.list_models().await.unwrap();
    assert_eq!(models, vec!["gpt-4o", "gpt-4o-mini"]);
}

#[tokio::test]
async fn test_github_complete_json() {
    let mock_server = MockServer::start().await;
    let response_str =
        r#"{"explanation":"github test","code":"print('github')","files":[],"commands":[],"checks":[],"caution":""}"#;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{"message": {"content": response_str}}]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::GitHub,
        mock_server.uri(),
        "gpt-4o".to_string(),
        30,
        "Be helpful.".to_string(),
        Some("test-key".to_string()),
        4096,
    );

    let reply = provider.complete_json("write code").await.unwrap();
    assert_eq!(reply.explanation, "github test");
    assert_eq!(reply.code, "print('github')");
}

#[tokio::test]
async fn test_github_chat_stream() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\ndata: {\"choices\":[{\"delta\":{\"content\":\" from GitHub\"}}]}\ndata: [DONE]\n"
        ))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::GitHub,
        mock_server.uri(),
        "gpt-4o".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let result = provider.complete_chat_stream("hi", "", "").await.unwrap();
    assert_eq!(result, "Hello from GitHub");
}

// ── xAI Grok ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_xai_list_models() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"id": "grok-2"}, {"id": "grok-2-vision"}]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::XAI,
        mock_server.uri(),
        "grok-2".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let models = provider.list_models().await.unwrap();
    assert_eq!(models, vec!["grok-2", "grok-2-vision"]);
}

#[tokio::test]
async fn test_xai_complete_json() {
    let mock_server = MockServer::start().await;
    let response_str =
        r#"{"explanation":"xai test","code":"print('xai')","files":[],"commands":[],"checks":[],"caution":""}"#;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{"message": {"content": response_str}}]
        })))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::XAI,
        mock_server.uri(),
        "grok-2".to_string(),
        30,
        "Be helpful.".to_string(),
        Some("test-key".to_string()),
        4096,
    );

    let reply = provider.complete_json("write code").await.unwrap();
    assert_eq!(reply.explanation, "xai test");
    assert_eq!(reply.code, "print('xai')");
}

#[tokio::test]
async fn test_xai_chat_stream() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\ndata: {\"choices\":[{\"delta\":{\"content\":\" from xAI\"}}]}\ndata: [DONE]\n"
        ))
        .mount(&mock_server)
        .await;

    let provider = Provider::new(
        ProviderKind::XAI,
        mock_server.uri(),
        "grok-2".to_string(),
        30,
        String::new(),
        Some("test-key".to_string()),
        4096,
    );

    let result = provider.complete_chat_stream("hi", "", "").await.unwrap();
    assert_eq!(result, "Hello from xAI");
}
