//! Your own rules, each with a stable id to refer to it by.
//!
//! An id is either chosen (`--id reply-lang`) or generated: `rule-` and four characters
//! from an alphabet without look-alikes (no 0, 1, i, l, o), about 920,000 combinations.
//! Generated ids are random rather than derived from the text, so editing a rule keeps
//! its id; uniqueness is checked against the existing rules.

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

const ALPHABET: &[u8] = b"23456789abcdefghjkmnpqrstuvwxyz";
const ID_LEN: usize = 4;
pub const MAX_ID: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalRule {
    pub id: String,
    pub text: String,
}

/// State written before 0.2.1 stored rules as plain strings. They get an id derived from
/// their text, so reads before the next write agree on it; the next write stores it.
impl<'de> Deserialize<'de> for LocalRule {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Text(String),
            Full { id: String, text: String },
        }
        Ok(match Repr::deserialize(d)? {
            Repr::Full { id, text } => LocalRule { id, text },
            Repr::Text(text) => LocalRule {
                id: format!("rule-{}", encode(&Sha256::digest(text.as_bytes()))),
                text,
            },
        })
    }
}

/// A new id that none of `existing` uses.
pub fn new_id(existing: &[LocalRule], text: &str) -> String {
    let mut n: u64 = 0;
    loop {
        let mut h = Sha256::new();
        h.update(text.as_bytes());
        h.update(std::process::id().to_le_bytes());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        h.update(now.to_le_bytes());
        h.update(n.to_le_bytes());
        let id = format!("rule-{}", encode(&h.finalize()));
        if !existing.iter().any(|r| r.id == id) {
            return id;
        }
        n += 1;
    }
}

/// Checks a chosen id: lowercase letters, digits and hyphens, starting with a letter or
/// digit, at most 32 characters, and not already in use.
pub fn check_id(id: &str, existing: &[LocalRule]) -> Result<(), String> {
    let valid = !id.is_empty()
        && id.len() <= MAX_ID
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !valid {
        return Err(format!(
            "invalid id \"{id}\": use lowercase letters, digits and hyphens, at most {MAX_ID} characters"
        ));
    }
    if existing.iter().any(|r| r.id == id) {
        return Err(format!(
            "a rule with id \"{id}\" already exists (see: handrail rule list)"
        ));
    }
    Ok(())
}

fn encode(bytes: &[u8]) -> String {
    bytes[..ID_LEN]
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str) -> LocalRule {
        LocalRule {
            id: id.into(),
            text: "x".into(),
        }
    }

    #[test]
    fn generated_ids_have_the_documented_shape_and_do_not_repeat() {
        let mut rules = Vec::new();
        for _ in 0..500 {
            let id = new_id(&rules, "same text");
            assert_eq!(id.len(), "rule-".len() + ID_LEN, "{id}");
            assert!(
                id["rule-".len()..].bytes().all(|b| ALPHABET.contains(&b)),
                "{id}"
            );
            rules.push(rule(&id));
        }
        let mut ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 500);
    }

    #[test]
    fn chosen_ids_are_validated() {
        let existing = vec![rule("reply-lang")];
        assert!(check_id("commit-style", &existing).is_ok());
        assert!(check_id("reply-lang", &existing)
            .unwrap_err()
            .contains("already exists"));
        for bad in [
            "",
            "Reply",
            "a b",
            "-x",
            "\u{4e2d}\u{6587}",
            &"a".repeat(33),
        ] {
            assert!(check_id(bad, &existing).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn plain_string_rules_from_older_state_get_a_stable_id() {
        let a: Vec<LocalRule> = serde_json::from_str(r#"["Reply in English"]"#).unwrap();
        let b: Vec<LocalRule> = serde_json::from_str(r#"["Reply in English"]"#).unwrap();
        assert_eq!(a, b);
        assert!(a[0].id.starts_with("rule-"));
        let full: Vec<LocalRule> =
            serde_json::from_str(r#"[{"id":"reply-lang","text":"Reply in English"}]"#).unwrap();
        assert_eq!(full[0].id, "reply-lang");
    }
}
