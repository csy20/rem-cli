use std::sync::LazyLock;

static BPE_CL100K: LazyLock<Option<tiktoken_rs::CoreBPE>> = LazyLock::new(|| tiktoken_rs::cl100k_base().ok());

/// Estimates token count, using tiktoken-rs if available, falling back to heuristic.
pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    if let Some(ref bpe) = *BPE_CL100K {
        let tokens = bpe.encode_with_special_tokens(text);
        return tokens.len().max(1);
    }
    estimate_tokens_heuristic(text)
}

fn estimate_tokens_heuristic(text: &str) -> usize {
    let bytes = text.len();
    let mut cjk_chars = 0usize;
    let mut other_unicode = 0usize;

    for c in text.chars() {
        let cp = c as u32;
        if c.is_ascii_whitespace() {
            continue;
        }
        if (0x4E00..=0x9FFF).contains(&cp)
            || (0x3400..=0x4DBF).contains(&cp)
            || (0x2E80..=0x2EFF).contains(&cp)
            || (0x3000..=0x303F).contains(&cp)
            || (0xF900..=0xFAFF).contains(&cp)
            || (0xFF00..=0xFFEF).contains(&cp)
        {
            cjk_chars += 1;
        } else if !c.is_ascii() {
            other_unicode += 1;
        }
    }

    let ascii_bytes = bytes - cjk_chars * 3 - other_unicode * 2;
    let ascii_tokens = if ascii_bytes > 0 {
        (ascii_bytes as f64 / 3.5).round() as usize
    } else {
        0
    };

    let total = ascii_tokens + cjk_chars + other_unicode * 2;
    total.max(1)
}

pub fn estimate_tokens_batch(texts: &[&str]) -> usize {
    texts.iter().map(|t| estimate_tokens(t)).sum()
}

pub fn context_usage_percent(used: usize, limit: usize) -> f64 {
    if limit == 0 {
        return 0.0;
    }
    (used as f64 / limit as f64 * 100.0).min(100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_zero() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn single_char_returns_at_least_one() {
        assert!(estimate_tokens("a") >= 1);
    }

    #[test]
    fn short_text_returns_positive() {
        assert!(estimate_tokens("hello world") >= 1);
    }

    #[test]
    fn cjk_characters_count_as_one() {
        let cjk = estimate_tokens("你好世界");
        let ascii = estimate_tokens("hello world a b c d");
        assert!(cjk <= ascii);
    }

    #[test]
    fn batch_counts_total() {
        let total = estimate_tokens_batch(&["hello", "world", "test"]);
        assert!(total > 0);
    }
}
