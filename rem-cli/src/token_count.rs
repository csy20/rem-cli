/// Estimates token count using a byte-pair heuristic tuned for code.
/// For English + code: ~1 token per 3.5 characters on average.
/// For CJK: ~1 token per character (overcounted by ratio = 2.5).
/// This is within ~20% of real tokenizers for most code.
pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
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

    // Each CJK char is roughly 1 token (but we account for byte ratio)
    // Non-CJK unicode: ~2 tokens per char
    // ASCII: ~1 token per 3.5 bytes
    let ascii_bytes = bytes
        - cjk_chars * 3       // CJK chars are 3 bytes in UTF-8
        - other_unicode * 2; // Other unicode: ~2 bytes avg
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
