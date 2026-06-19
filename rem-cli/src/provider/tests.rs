use super::*;
use crate::ModelReply;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_api_url_without_api_suffix() {
    let url = api_url("http://localhost:11434", "tags");
    assert_eq!(url, "http://localhost:11434/api/tags");
}

#[tokio::test]
async fn test_api_url_with_api_suffix() {
    let url = api_url("http://localhost:11434/api", "tags");
    assert_eq!(url, "http://localhost:11434/api/tags");
}

#[tokio::test]
async fn test_api_url_trailing_slash() {
    let url = api_url("http://localhost:11434/", "generate");
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
    assert_eq!(ProviderKind::from_str("unknown"), ProviderKind::Ollama);
}

#[tokio::test]
async fn test_provider_kind_as_str() {
    assert_eq!(ProviderKind::Ollama.as_str(), "ollama");
    assert_eq!(ProviderKind::OpenAI.as_str(), "openai");
    assert_eq!(ProviderKind::Gemini.as_str(), "gemini");
    assert_eq!(ProviderKind::Anthropic.as_str(), "anthropic");
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
        .and(path("/api/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "{\"response\":\"Hello\",\"done\":false}\n{\"response\":\" World\",\"done\":false}\n{\"response\":\"\",\"done\":true}\n"
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
    let response_str = r#"{"explanation":"test","code":"print('hello')","files":[],"commands":[],"checks":[],"caution":""}"#;
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
    let response_str =
        r#"{"explanation":"test","code":"","files":[],"commands":[],"checks":[],"caution":""}"#;
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
    let response_str =
        r#"{"explanation":"test","code":"","files":[],"commands":[],"checks":[],"caution":""}"#;
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
