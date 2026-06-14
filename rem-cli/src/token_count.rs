pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let mut total = 0usize;
    let mut ascii_run = 0usize;
    for c in text.chars() {
        if c.is_ascii() {
            ascii_run += 1;
        } else {
            total += ascii_run / 4;
            ascii_run = 0;
            if (c as u32) > 0x4E00 && (c as u32) < 0x9FFF {
                total += 1;
            } else {
                total += 2;
            }
        }
    }
    total += ascii_run / 4;
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
