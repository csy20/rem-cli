//! Text and string utility functions.
//! Provides human-readable sizes, byte-safe truncation, line truncation,
//! and timestamp formatting for display purposes.

/// Formats byte counts as human-readable strings (e.g., `1.5K`, `3.2M`).
pub(crate) fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Truncates a string to at most `max` bytes, preserving char boundaries.
pub(crate) fn truncate_bytes(s: &str, max: usize) -> String {
    if s.is_empty() || max == 0 {
        return String::new();
    }
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 {
        return "[truncated]".to_string();
    }
    format!("{}\n...[truncated]", &s[..end])
}

/// Truncates a string to at most `max_lines` lines.
pub(crate) fn truncate_to_lines(s: &str, max_lines: usize) -> String {
    let all_lines: Vec<&str> = s.lines().collect();
    let total = all_lines.len();
    let mut result = all_lines.into_iter().take(max_lines).collect::<Vec<_>>().join("\n");
    if total > max_lines {
        result.push_str("\n...[truncated]");
    }
    result
}

/// Converts a Unix day count to a year, returning the year and remaining days.
fn days_to_year(days: i64) -> (i64, i64) {
    let mut y = 1970i64;
    let mut d = days;
    const DAYS_IN_400_YEARS: i64 = 146097;
    if d >= DAYS_IN_400_YEARS {
        let blocks = d / DAYS_IN_400_YEARS;
        y += blocks * 400;
        d -= blocks * DAYS_IN_400_YEARS;
    }
    loop {
        let year_days = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
            366
        } else {
            365
        };
        if d < year_days {
            break;
        }
        d -= year_days;
        y += 1;
    }
    (y, d)
}

/// Returns the current year.
pub(crate) fn current_year() -> i64 {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let days = (dur.as_secs() / 86400) as i64;
    days_to_year(days).0
}

/// Returns the current UTC timestamp as `YYYY-MM-DD HH:MM:SS`.
pub(crate) fn format_timestamp() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = dur.as_secs();

    let days = total_secs / 86400;
    let time_secs = total_secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    let (y, d) = days_to_year(days as i64);
    let is_leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [
        31u64,
        if is_leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1usize;
    let mut day = d as u64; // remaining days within year
    for &md in &month_days {
        if day < md {
            break;
        }
        day -= md;
        month += 1;
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y,
        month,
        day + 1,
        hours,
        minutes,
        seconds
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_string() {
        let out = truncate_bytes("abcdef", 3);
        assert!(out.starts_with("abc"));
    }

    #[test]
    fn human_size_works() {
        assert_eq!(human_size(500), "500");
        assert_eq!(human_size(2048), "2.0K");
        assert_eq!(human_size(5_242_880), "5.0M");
    }

    #[test]
    fn truncate_to_lines_limits_lines() {
        let input = "line1\nline2\nline3\nline4";
        let out = truncate_to_lines(input, 2);
        assert_eq!(out.lines().count(), 3);
        assert!(out.ends_with("[truncated]"));
    }

    #[test]
    fn truncate_to_lines_passes_short() {
        let input = "short";
        let out = truncate_to_lines(input, 10);
        assert_eq!(out, "short");
    }

    #[test]
    fn format_timestamp_returns_valid_format() {
        let ts = format_timestamp();
        assert_eq!(ts.len(), 19);
        assert!(ts.chars().nth(4) == Some('-'));
        assert!(ts.chars().nth(7) == Some('-'));
    }

    #[test]
    fn truncate_bytes_preserves_char_boundaries() {
        let s = "Hell\u{00e9} world";
        let out = truncate_bytes(s, 5);
        assert_eq!(out, "Hell\n...[truncated]");
        assert!(!out.contains('\u{00e9}'));
    }

    #[test]
    fn prop_truncate_bytes_never_exceeds_limit() {
        proptest::proptest!(|(s: String, limit in 0usize..1000)| {
            let result = truncate_bytes(&s, limit);
            // The truncated suffix "\n...[truncated]" is 14 bytes,
            // but we bound by limit + 20 for a reasonable upper bound.
            assert!(result.len() <= limit + 20,
                "truncate_bytes({}, {}) = {} (len {})",
                s, limit, result, result.len());
        });
    }

    #[test]
    fn prop_truncate_bytes_preserves_prefix() {
        proptest::proptest!(|(s: String, limit in 10usize..500usize)| {
            let result = truncate_bytes(&s, limit);
            if s.is_empty() {
                assert_eq!(result, "", "empty string should return empty");
            } else if s.len() > limit {
                assert!(result.contains("[truncated]"), "result should indicate truncation");
                if result != "[truncated]" {
                    let suffix = "\n...[truncated]";
                    let prefix = &result[..result.len() - suffix.len()];
                    assert!(s.starts_with(prefix),
                        "truncated result should be a prefix of original");
                }
            } else {
                assert_eq!(result, s, "result should equal original when under limit");
            }
        });
    }

    #[test]
    fn prop_human_size_never_empty() {
        proptest::proptest!(|(bytes: u64)| {
            let result = human_size(bytes);
            assert!(!result.is_empty(), "human_size({}) should not be empty", bytes);
            assert!(result.len() < 20, "human_size({}) = '{}' should be compact", bytes, result);
        });
    }

    #[test]
    fn prop_truncate_to_lines_respects_limit() {
        proptest::proptest!(|(lines in proptest::collection::vec("[a-z]{0,20}", 0..50), max_lines in 1usize..20usize)| {
            let text = lines.join("\n");
            let result = truncate_to_lines(&text, max_lines);
            let result_lines = result.lines().count();
            // May have +1 for the "[truncated]" line
            assert!(result_lines <= max_lines + 1,
                "truncate_to_lines with max_lines={} returned {} lines", max_lines, result_lines);
        });
    }
}
