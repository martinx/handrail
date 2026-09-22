//! Dotted numeric versions, as printed by `claude --version` ("2.1.278 (Claude Code)").
//!
//! Only the leading numeric components are compared. That is all `min_version` needs, and
//! it avoids pulling in a semver parser for strings that are not always semver.

use std::cmp::Ordering;

/// Parses the first run of dotted numbers in `s`: "2.1.278 (Claude Code)" -> [2, 1, 278].
pub fn parse(s: &str) -> Option<Vec<u64>> {
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let token: String = s[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let parts: Option<Vec<u64>> = token
        .trim_end_matches('.')
        .split('.')
        .map(|p| p.parse().ok())
        .collect();
    parts.filter(|p| !p.is_empty())
}

/// Compares two versions component by component; missing components count as zero.
pub fn compare(a: &[u64], b: &[u64]) -> Ordering {
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        match x.cmp(&y) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

/// True when `installed` is at least `required`.
pub fn at_least(installed: &str, required: &str) -> Option<bool> {
    Some(compare(&parse(installed)?, &parse(required)?) != Ordering::Less)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_code_output() {
        assert_eq!(parse("2.1.278 (Claude Code)"), Some(vec![2, 1, 278]));
        assert_eq!(parse("v1.2"), Some(vec![1, 2]));
        assert_eq!(parse("no digits"), None);
    }

    #[test]
    fn compares_numerically_not_lexically() {
        assert_eq!(at_least("2.1.278", "2.1.242"), Some(true));
        assert_eq!(at_least("2.1.99", "2.1.242"), Some(false));
        assert_eq!(at_least("2.1", "2.1.0"), Some(true));
        assert_eq!(at_least("3", "2.9.9"), Some(true));
    }
}
