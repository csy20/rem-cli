//! SigNoz MCP client (HTTP transport).
//!
//! Talks to the self-hosted or Cloud SigNoz MCP server using the MCP
//! Streamable HTTP / JSON-RPC protocol. Used by `/observe` to pull live
//! router-agent traces and feed them into the active LLM.
//!
//! Endpoint defaults (self-host via Foundry with `mcp.enabled: true`):
//!   SIGNOZ_MCP_URL = http://localhost:8000/mcp
//! Auth (optional for local stack without OAuth):
//!   SIGNOZ_API_KEY header when the server requires it
//!   SIGNOZ_URL for the SigNoz UI/API base (header X-SigNoz-URL)

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Configuration for the SigNoz MCP client.
#[derive(Debug, Clone)]
pub struct SignozConfig {
    /// MCP HTTP endpoint, e.g. `http://localhost:8000/mcp`
    pub mcp_url: String,
    /// Optional service-account API key (`SIGNOZ-API-KEY` header).
    pub api_key: Option<String>,
    /// Optional SigNoz instance URL (`X-SigNoz-URL` header for multi-tenant HTTP).
    pub signoz_url: Option<String>,
    /// Default service filter for router-agent demos.
    pub service_name: String,
    /// Relative lookback window for trace queries.
    pub time_range: String,
}

impl Default for SignozConfig {
    fn default() -> Self {
        Self {
            mcp_url: std::env::var("SIGNOZ_MCP_URL").unwrap_or_else(|_| "http://localhost:8000/mcp".into()),
            api_key: std::env::var("SIGNOZ_API_KEY").ok().filter(|s| !s.is_empty()),
            signoz_url: std::env::var("SIGNOZ_URL").ok().filter(|s| !s.is_empty()),
            service_name: std::env::var("SIGNOZ_SERVICE_NAME").unwrap_or_else(|_| "router-agent".into()),
            time_range: std::env::var("SIGNOZ_TIME_RANGE").unwrap_or_else(|_| "6h".into()),
        }
    }
}

impl SignozConfig {
    /// Build from rem AppConfig fields + env fallbacks.
    pub fn from_app(
        mcp_url: Option<&str>,
        api_key: Option<&str>,
        signoz_url: Option<&str>,
        service_name: Option<&str>,
    ) -> Self {
        let mut c = Self::default();
        if let Some(u) = mcp_url.filter(|s| !s.is_empty()) {
            c.mcp_url = u.to_string();
        }
        if let Some(k) = api_key.filter(|s| !s.is_empty()) {
            c.api_key = Some(k.to_string());
        }
        if let Some(u) = signoz_url.filter(|s| !s.is_empty()) {
            c.signoz_url = Some(u.to_string());
        }
        if let Some(s) = service_name.filter(|x| !x.is_empty()) {
            c.service_name = s.to_string();
        }
        c
    }
}

/// Lightweight MCP JSON-RPC client for SigNoz.
pub struct SignozClient {
    http: Client,
    cfg: SignozConfig,
    next_id: AtomicU64,
    session_id: Option<String>,
    initialized: bool,
}

impl SignozClient {
    pub fn new(cfg: SignozConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent("rem-cli-signoz-mcp/0.1")
            .build()
            .context("build reqwest client for SigNoz MCP")?;
        Ok(Self {
            http,
            cfg,
            next_id: AtomicU64::new(1),
            session_id: None,
            initialized: false,
        })
    }

    pub fn config(&self) -> &SignozConfig {
        &self.cfg
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn apply_headers(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut rb = rb
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        if let Some(ref key) = self.cfg.api_key {
            // Self-hosted MCP accepts either header (docs: SIGNOZ-API-KEY or Authorization).
            rb = rb
                .header("SIGNOZ-API-KEY", key)
                .header("Authorization", format!("Bearer {key}"));
        }
        // Cloud multi-tenant MCP may need X-SigNoz-URL. Self-hosted Foundry MCP
        // already has SIGNOZ_URL on the server; sending localhost is rejected.
        if let Some(ref url) = self.cfg.signoz_url {
            let lower = url.to_lowercase();
            let is_local = lower.contains("localhost") || lower.contains("127.0.0.1") || lower.contains("0.0.0.0");
            if !is_local {
                rb = rb.header("X-SigNoz-URL", url);
            }
        }
        if let Some(ref sid) = self.session_id {
            rb = rb.header("Mcp-Session-Id", sid);
        }
        rb
    }

    /// POST a JSON-RPC request; handles plain JSON and SSE (`data: ...`) bodies.
    async fn rpc(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_id();
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params.unwrap_or(json!({})),
        });

        let resp = self
            .apply_headers(self.http.post(&self.cfg.mcp_url))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("MCP POST {method} to {}", self.cfg.mcp_url))?;

        // Capture session id if the server issues one
        if let Some(sid) = resp.headers().get("mcp-session-id") {
            if let Ok(s) = sid.to_str() {
                self.session_id = Some(s.to_string());
            }
        }

        let status = resp.status();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let text = resp.text().await.context("read MCP response body")?;

        if !status.is_success() {
            return Err(anyhow!("MCP {method} HTTP {status}: {}", truncate(&text, 500)));
        }

        let parsed =
            parse_mcp_body(&text, &content_type).with_context(|| format!("parse MCP response for {method}"))?;

        if let Some(err) = parsed.get("error") {
            return Err(anyhow!("MCP error on {method}: {err}"));
        }
        Ok(parsed.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Initialize the MCP session (protocol handshake).
    pub async fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "rem-cli",
                "version": env!("CARGO_PKG_VERSION"),
            }
        });
        let _ = self.rpc("initialize", Some(params)).await?;

        // notifications/initialized (no response expected; some servers 202)
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        });
        let _ = self
            .apply_headers(self.http.post(&self.cfg.mcp_url))
            .json(&notif)
            .send()
            .await;

        self.initialized = true;
        Ok(())
    }

    /// List tools advertised by the MCP server.
    pub async fn list_tools(&mut self) -> Result<Vec<String>> {
        self.initialize().await?;
        let result = self.rpc("tools/list", Some(json!({}))).await?;
        let tools = result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(tools
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
            .collect())
    }

    /// Call a named MCP tool with JSON arguments.
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        self.initialize().await?;
        let mut args = arguments;
        // searchContext is required/accepted by all SigNoz tools for MCP observability
        if args.get("searchContext").is_none() {
            if let Some(obj) = args.as_object_mut() {
                obj.insert("searchContext".into(), json!(format!("rem-cli observe tool={name}")));
            }
        }
        let params = json!({
            "name": name,
            "arguments": args,
        });
        let result = self.rpc("tools/call", Some(params)).await?;
        Ok(extract_tool_text(result))
    }

    /// High-level: gather router-agent observability context for a natural-language query.
    pub async fn observe_context(&mut self, query: &str) -> Result<String> {
        self.initialize().await?;

        let q = query.to_lowercase();
        let service = self.cfg.service_name.clone();
        let time_range = self.cfg.time_range.clone();

        let mut sections: Vec<String> = Vec::new();
        sections.push(format!(
            "# SigNoz observe context\nquery: {query}\nservice: {service}\ntime_range: {time_range}\nmcp: {}\n",
            self.cfg.mcp_url
        ));

        // 1) Services
        match self
            .call_tool(
                "signoz_list_services",
                json!({
                    "timeRange": time_range,
                    "limit": 50,
                    "searchContext": query,
                }),
            )
            .await
        {
            Ok(v) => sections.push(format!("## Services\n{}\n", pretty_trunc(&v, 4000))),
            Err(e) => sections.push(format!("## Services\n(error: {e})\n")),
        }

        // 2) Aggregate traces for the service
        match self
            .call_tool(
                "signoz_aggregate_traces",
                json!({
                    "aggregation": "count",
                    "groupBy": "service.name",
                    "filter": format!("service.name = '{}'", service),
                    "timeRange": time_range,
                    "searchContext": query,
                }),
            )
            .await
        {
            Ok(v) => sections.push(format!(
                "## Trace aggregates (by service)\n{}\n",
                pretty_trunc(&v, 4000)
            )),
            Err(e) => sections.push(format!("## Trace aggregates\n(error: {e})\n")),
        }

        // 3) Search spans — bias filters from query keywords
        let filter = build_trace_filter(&q, &service);
        match self
            .call_tool(
                "signoz_search_traces",
                json!({
                    "filter": filter,
                    "timeRange": time_range,
                    "limit": 30,
                    "searchContext": query,
                }),
            )
            .await
        {
            Ok(v) => {
                sections.push(format!(
                    "## Spans matching filter `{filter}`\n{}\n",
                    pretty_trunc(&v, 4000)
                ));
                // If slowest requested, also aggregate p99 by name
            }
            Err(e) => sections.push(format!("## Spans\n(error: {e})\n")),
        }

        // 4) Stage-level aggregates when escalation/fireworks mentioned
        if q.contains("fireworks")
            || q.contains("escalat")
            || q.contains("stage")
            || q.contains("waterfall")
            || q.contains("token")
        {
            match self
                .call_tool(
                    "signoz_aggregate_traces",
                    json!({
                        "aggregation": "count",
                        "groupBy": "stage",
                        "filter": format!("service.name = '{}' AND name = 'task.route'", service),
                        "timeRange": time_range,
                        "searchContext": query,
                    }),
                )
                .await
            {
                Ok(v) => sections.push(format!("## task.route stages\n{}\n", pretty_trunc(&v, 4000))),
                Err(e) => sections.push(format!("## task.route stages\n(error: {e})\n")),
            }

            match self
                .call_tool(
                    "signoz_search_traces",
                    json!({
                        "filter": format!(
                            "service.name = '{}' AND name = 'llm.fireworks_call'",
                            service
                        ),
                        "timeRange": time_range,
                        "limit": 20,
                        "searchContext": query,
                    }),
                )
                .await
            {
                Ok(v) => sections.push(format!("## Fireworks LLM spans\n{}\n", pretty_trunc(&v, 8000))),
                Err(e) => sections.push(format!("## Fireworks LLM spans\n(error: {e})\n")),
            }
        }

        // 5) Slowest / latency
        if q.contains("slow") || q.contains("latency") || q.contains("p99") || q.contains("longest") {
            match self
                .call_tool(
                    "signoz_aggregate_traces",
                    json!({
                        "aggregation": "p99",
                        "aggregateOn": "durationNano",
                        "groupBy": "name",
                        "filter": format!("service.name = '{}'", service),
                        "timeRange": time_range,
                        "limit": 20,
                        "searchContext": query,
                    }),
                )
                .await
            {
                Ok(v) => sections.push(format!("## Slowest operations (p99)\n{}\n", pretty_trunc(&v, 4000))),
                Err(e) => sections.push(format!("## Slowest operations\n(error: {e})\n")),
            }
        }

        // 6) If a specific task id is mentioned, search for it
        if let Some(tid) = extract_task_id(query) {
            match self
                .call_tool(
                    "signoz_search_traces",
                    json!({
                        "filter": format!(
                            "service.name = '{}' AND task_id = '{}'",
                            service, tid
                        ),
                        "timeRange": time_range,
                        "limit": 20,
                        "searchContext": query,
                    }),
                )
                .await
            {
                Ok(v) => sections.push(format!("## Spans for task_id={tid}\n{}\n", pretty_trunc(&v, 8000))),
                Err(e) => sections.push(format!("## Spans for task_id={tid}\n(error: {e})\n")),
            }
        }

        // Keep context LLM-friendly (small local models choke on huge JSON dumps).
        let joined = sections.join("\n");
        const MAX_CTX: usize = 12_000;
        if joined.len() > MAX_CTX {
            Ok(format!(
                "{}\n\n…[truncated {} → {} chars for LLM context window]",
                &joined[..MAX_CTX],
                joined.len(),
                MAX_CTX
            ))
        } else {
            Ok(joined)
        }
    }
}

fn build_trace_filter(q: &str, service: &str) -> String {
    let mut parts = vec![format!("service.name = '{}'", service)];
    if q.contains("fireworks") {
        parts.push("(name = 'llm.fireworks_call' OR stage = 'fireworks')".into());
    } else if q.contains("local") {
        parts.push("(name = 'llm.local_generate' OR stage = 'local_llm')".into());
    } else if q.contains("classify") {
        parts.push("name = 'task.classify'".into());
    } else if q.contains("free") {
        parts.push("stage = 'free_solver'".into());
    } else {
        parts.push("name = 'task.process'".into());
    }
    parts.join(" AND ")
}

fn extract_task_id(query: &str) -> Option<String> {
    // "task 3" / "task t3" / "task_id=t3" / bare "t3"
    let re = regex::Regex::new(r"(?i)task[_\s-]*(?:id)?[_\s:=]*([a-zA-Z0-9_-]+)|\b(t\d+)\b").ok()?;
    if let Some(c) = re.captures(query) {
        if let Some(m) = c.get(1) {
            let s = m.as_str();
            // Normalize bare digits from "task 3" → "t3"
            if s.chars().all(|ch| ch.is_ascii_digit()) {
                return Some(format!("t{s}"));
            }
            return Some(s.to_string());
        }
        if let Some(m) = c.get(2) {
            return Some(m.as_str().to_string());
        }
    }
    None
}

/// Parse MCP HTTP body — either a single JSON object or SSE lines (`data: {...}`).
fn parse_mcp_body(text: &str, content_type: &str) -> Result<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    if content_type.contains("text/event-stream") || trimmed.starts_with("event:") || trimmed.contains("\ndata:") {
        // Take the last JSON payload from data: lines
        let mut last: Option<Value> = None;
        for line in trimmed.lines() {
            let line = line.trim();
            if let Some(payload) = line.strip_prefix("data:") {
                let payload = payload.trim();
                if payload.is_empty() || payload == "[DONE]" {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(payload) {
                    last = Some(v);
                }
            }
        }
        return last.ok_or_else(|| anyhow!("no data: frames in SSE body"));
    }
    serde_json::from_str(trimmed).context("JSON parse MCP body")
}

/// Collapse MCP tool result content blocks into a single JSON Value.
fn extract_tool_text(result: Value) -> Value {
    // Shape: { content: [ { type: "text", text: "..." } ], isError?: bool }
    if let Some(arr) = result.get("content").and_then(|c| c.as_array()) {
        let mut texts = Vec::new();
        for item in arr {
            if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                texts.push(t.to_string());
            }
        }
        if texts.len() == 1 {
            // Prefer structured JSON if the text is JSON
            if let Ok(v) = serde_json::from_str::<Value>(&texts[0]) {
                return v;
            }
            return Value::String(texts[0].clone());
        }
        if !texts.is_empty() {
            return Value::String(texts.join("\n"));
        }
    }
    result
}

fn pretty_trunc(v: &Value, max: usize) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    };
    truncate(&s, max)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…\n[truncated {} bytes]", &s[..max], s.len() - max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_id_extract() {
        assert_eq!(extract_task_id("why did task 3 escalate").as_deref(), Some("t3"));
        assert_eq!(extract_task_id("show t4 spans").as_deref(), Some("t4"));
    }

    #[test]
    fn parse_json_body() {
        let v = parse_mcp_body(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#, "application/json").unwrap();
        assert_eq!(v["result"]["ok"], true);
    }

    #[test]
    fn parse_sse_body() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"tools\":[]}}\n\n";
        let v = parse_mcp_body(body, "text/event-stream").unwrap();
        assert!(v["result"]["tools"].is_array());
    }
}
