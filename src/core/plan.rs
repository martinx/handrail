//! A plan: the exact file changes an operation will make, relative to one root directory.
//!
//! Every change carries a precondition: what the file looked like when the plan was made.
//! Between planning (unprivileged, shown to the user) and applying (privileged), the
//! world can change. If it did, applying refuses rather than overwrite something the user
//! never reviewed.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;

/// What the target file must look like right before the change is made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Expect {
    /// The file must not exist.
    Absent,
    /// The file must exist with exactly this SHA-256 (hex).
    Hash(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Create or replace a file.
    Write {
        path: String,
        content: Vec<u8>,
        mode: u32,
        expect: Expect,
    },
    /// Remove a file.
    Delete { path: String, expect: Expect },
    /// Remove a directory if it is empty; never fails if it is not.
    RemoveDirIfEmpty { path: String },
}

impl Op {
    pub fn path(&self) -> &str {
        match self {
            Op::Write { path, .. } | Op::Delete { path, .. } | Op::RemoveDirIfEmpty { path } => {
                path
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub ops: Vec<Op>,
}

/// The stable, content-addressed description of a plan that its hash is computed over.
#[derive(Serialize)]
struct Canonical<'a> {
    kind: &'static str,
    path: &'a str,
    content_sha256: Option<String>,
    mode: Option<u32>,
    expect: Option<&'a Expect>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// SHA-256 over every op, including content hashes and preconditions.
    ///
    /// The privileged step recomputes the plan itself and compares this hash with the one
    /// the user reviewed; any difference aborts the run.
    pub fn hash(&self) -> String {
        let canon: Vec<Canonical> = self
            .ops
            .iter()
            .map(|op| match op {
                Op::Write {
                    path,
                    content,
                    mode,
                    expect,
                } => Canonical {
                    kind: "write",
                    path,
                    content_sha256: Some(sha256_hex(content)),
                    mode: Some(*mode),
                    expect: Some(expect),
                },
                Op::Delete { path, expect } => Canonical {
                    kind: "delete",
                    path,
                    content_sha256: None,
                    mode: None,
                    expect: Some(expect),
                },
                Op::RemoveDirIfEmpty { path } => Canonical {
                    kind: "rmdir",
                    path,
                    content_sha256: None,
                    mode: None,
                    expect: None,
                },
            })
            .collect();
        sha256_hex(&serde_json::to_vec(&canon).expect("plan serialises"))
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// The precondition for `rel` as it is on disk right now.
pub fn current(root: &Path, rel: &str) -> Expect {
    match std::fs::read(root.join(rel)) {
        Ok(bytes) => Expect::Hash(sha256_hex(&bytes)),
        Err(_) => Expect::Absent,
    }
}

/// A relative path that stays inside the root: no absolute paths, no `..`.
pub fn is_safe_relative(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &str, content: &str) -> Op {
        Op::Write {
            path: path.into(),
            content: content.as_bytes().to_vec(),
            mode: 0o644,
            expect: Expect::Absent,
        }
    }

    #[test]
    fn hash_changes_with_content_mode_and_precondition() {
        let a = Plan {
            ops: vec![write("x", "1")],
        };
        let b = Plan {
            ops: vec![write("x", "2")],
        };
        let mut c = a.clone();
        if let Op::Write { mode, .. } = &mut c.ops[0] {
            *mode = 0o755;
        }
        let mut d = a.clone();
        if let Op::Write { expect, .. } = &mut d.ops[0] {
            *expect = Expect::Hash("00".into());
        }
        let hashes = [a.hash(), b.hash(), c.hash(), d.hash()];
        for i in 0..hashes.len() {
            for j in i + 1..hashes.len() {
                assert_ne!(hashes[i], hashes[j], "{i} vs {j}");
            }
        }
        assert_eq!(a.hash(), a.clone().hash());
    }

    #[test]
    fn rejects_paths_that_escape_the_root() {
        assert!(is_safe_relative("managed-settings.d/handrail-privacy.json"));
        assert!(!is_safe_relative("../etc/passwd"));
        assert!(!is_safe_relative("/etc/passwd"));
        assert!(!is_safe_relative("a/../../b"));
        assert!(!is_safe_relative(""));
    }
}
