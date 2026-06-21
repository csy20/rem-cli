use serde::{Deserialize, Serialize};

/// Effort level for reasoning models (OpenAI o1/o3).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "low" => ReasoningEffort::Low,
            "high" => ReasoningEffort::High,
            _ => ReasoningEffort::Medium,
        }
    }
}

/// Configuration for thinking/reasoning models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningConfig {
    /// Whether to enable reasoning/thinking mode.
    pub enabled: bool,
    /// Reasoning effort for OpenAI o1/o3 (low/medium/high).
    pub effort: ReasoningEffort,
    /// Token budget for Anthropic extended thinking.
    pub thinking_budget: u32,
    /// Whether to show the reasoning trace to the user.
    pub show_reasoning: bool,
}

impl Default for ReasoningConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            effort: ReasoningEffort::Medium,
            thinking_budget: 8192,
            show_reasoning: false,
        }
    }
}

/// Detects if a model name is a reasoning model that needs special handling.
pub fn is_reasoning_model(model: &str) -> bool {
    let lower = model.to_lowercase();
    lower.starts_with("o1-")
        || lower.starts_with("o3-")
        || lower.contains("deepseek-r1")
        || lower.contains("claude-sonnet-4-20") && lower.contains("thinking")
        || lower.contains("thinking")
}

/// Detects if a model requires disabling streaming (e.g., o1-preview).
pub fn requires_non_streaming(model: &str) -> bool {
    let lower = model.to_lowercase();
    lower == "o1-preview" || lower == "o1-mini"
}

/// Checks if a model does NOT support system prompts (o1/o3).
pub fn system_prompt_not_supported(model: &str) -> bool {
    let lower = model.to_lowercase();
    lower.starts_with("o1-") || lower.starts_with("o3-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_reasoning_models() {
        assert!(is_reasoning_model("o1-preview"));
        assert!(is_reasoning_model("o1-mini"));
        assert!(is_reasoning_model("o3-mini"));
        assert!(is_reasoning_model("deepseek-r1"));
        assert!(is_reasoning_model("deepseek-r1:671b"));
        assert!(!is_reasoning_model("gpt-4o"));
        assert!(!is_reasoning_model("claude-sonnet-4-20250514"));
    }

    #[test]
    fn test_requires_non_streaming() {
        assert!(requires_non_streaming("o1-preview"));
        assert!(requires_non_streaming("o1-mini"));
        assert!(!requires_non_streaming("o3-mini"));
        assert!(!requires_non_streaming("gpt-4o"));
    }

    #[test]
    fn test_system_prompt_not_supported() {
        assert!(system_prompt_not_supported("o1-preview"));
        assert!(system_prompt_not_supported("o3-mini"));
        assert!(!system_prompt_not_supported("gpt-4o"));
    }

    #[test]
    fn test_reasoning_effort_roundtrip() {
        assert_eq!(ReasoningEffort::from_str("low"), ReasoningEffort::Low);
        assert_eq!(ReasoningEffort::from_str("medium"), ReasoningEffort::Medium);
        assert_eq!(ReasoningEffort::from_str("high"), ReasoningEffort::High);
        assert_eq!(ReasoningEffort::from_str("unknown"), ReasoningEffort::Medium);
    }

    #[test]
    fn test_reasoning_config_default() {
        let cfg = ReasoningConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.effort, ReasoningEffort::Medium);
        assert_eq!(cfg.thinking_budget, 8192);
    }
}
