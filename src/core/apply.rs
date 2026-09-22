//! Applying a plan so that an interrupted run leaves either the old state or the new one.
//!
//! The M0 shell prototype once stopped halfway: settings written, instructions not. On a
//! policy directory that is the worst outcome, because it looks installed. The sequence:
//!
//! 1. **Check** every precondition. Anything changed since planning: refuse, touch nothing.
//! 2. **Validate** every JSON file. A managed settings file that is not valid JSON makes
//!    Claude Code refuse to start.
//! 3. **Stage** new contents inside the root (same filesystem, so renames are atomic) and fsync.
//! 4. **Back up** every file that will be replaced or removed.
//! 5. **Journal**: record what existed before and where the backup is, and fsync.
//! 6. **Commit**: rename staged files into place, delete, prune empty directories.
//! 7. Remove the journal. Only now is the run complete.
//!
//! If the process dies anywhere in step 6, the journal remains. [`recover`] restores
//! every file from the backup and removes files that did not exist before. The next
//! command runs it automatically.

use crate::core::plan::{current, is_safe_relative, Op, Plan};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const JOURNAL: &str = ".handrail-journal.json";
pub const STAGING: &str = ".handrail-staging";

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("{path} changed since the plan was made; nothing was modified. Run the command again to review the new plan.")]
    Changed { path: String },
    #[error("{path}: refusing to write outside the target directory")]
    UnsafePath { path: String },
    #[error("{path} is not valid JSON ({reason}); nothing was modified")]
    InvalidJson { path: String, reason: String },
    #[error("an earlier run was interrupted; recover it first (journal at {0})")]
    Interrupted(PathBuf),
    #[error("simulated failure after {0} operations (test only)")]
    Simulated(usize),
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
}

fn io<T>(r: std::io::Result<T>, context: impl FnOnce() -> String) -> Result<T, ApplyError> {
    r.map_err(|source| ApplyError::Io {
        context: context(),
        source,
    })
}

#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    backup_dir: PathBuf,
    /// Every file the commit may touch, and whether it existed before.
    files: Vec<JournalEntry>,
    /// Directories the commit may create; removed on recovery if left empty.
    created_dirs: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JournalEntry {
    path: String,
    existed: bool,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub written: usize,
    pub deleted: usize,
}

/// Applies `plan` under `root`, backing up replaced files into `backup_dir`.
pub fn apply(root: &Path, plan: &Plan, backup_dir: &Path) -> Result<Outcome, ApplyError> {
    apply_with_failpoint(root, plan, backup_dir, None)
}

/// Like [`apply`], but stops dead after `fail_after` commit operations, leaving the
/// journal behind exactly as a crash would. Used by tests.
pub fn apply_with_failpoint(
    root: &Path,
    plan: &Plan,
    backup_dir: &Path,
    fail_after: Option<usize>,
) -> Result<Outcome, ApplyError> {
    let journal_path = root.join(JOURNAL);
    if journal_path.exists() {
        return Err(ApplyError::Interrupted(journal_path));
    }

    // 1. Preconditions and path safety
    for op in &plan.ops {
        if !is_safe_relative(op.path()) {
            return Err(ApplyError::UnsafePath {
                path: op.path().to_string(),
            });
        }
        let expect = match op {
            Op::Write { expect, .. } | Op::Delete { expect, .. } => expect,
            Op::RemoveDirIfEmpty { .. } => continue,
        };
        if &current(root, op.path()) != expect {
            return Err(ApplyError::Changed {
                path: op.path().to_string(),
            });
        }
    }

    // 2. Validate content
    for op in &plan.ops {
        if let Op::Write { path, content, .. } = op {
            if path.ends_with(".json") {
                if let Err(e) = serde_json::from_slice::<serde_json::Value>(content) {
                    return Err(ApplyError::InvalidJson {
                        path: path.clone(),
                        reason: e.to_string(),
                    });
                }
            }
        }
    }

    // 3. Stage
    let staging = root.join(STAGING);
    if staging.exists() {
        io(fs::remove_dir_all(&staging), || {
            format!("removing stale {}", staging.display())
        })?;
    }
    io(fs::create_dir_all(&staging), || {
        format!("creating {}", staging.display())
    })?;
    for (i, op) in plan.ops.iter().enumerate() {
        if let Op::Write { content, mode, .. } = op {
            let p = staging.join(i.to_string());
            write_synced(&p, content, *mode)?;
        }
    }

    // 4. Back up what will be replaced or removed
    let mut files = Vec::new();
    for op in &plan.ops {
        let (Op::Write { path, .. } | Op::Delete { path, .. }) = op else {
            continue;
        };
        let src = root.join(path);
        let existed = src.exists();
        if existed {
            let dst = backup_dir.join(path);
            if let Some(parent) = dst.parent() {
                io(fs::create_dir_all(parent), || {
                    format!("creating {}", parent.display())
                })?;
            }
            io(fs::copy(&src, &dst), || {
                format!("backing up {}", src.display())
            })?;
        }
        files.push(JournalEntry {
            path: path.clone(),
            existed,
        });
    }
    let created_dirs = missing_parents(root, plan);

    // 5. Journal
    let journal = Journal {
        backup_dir: backup_dir.to_path_buf(),
        files,
        created_dirs,
    };
    write_synced(
        &journal_path,
        &serde_json::to_vec_pretty(&journal).expect("journal serialises"),
        0o644,
    )?;

    // 6. Commit
    let mut outcome = Outcome::default();
    for (i, op) in plan.ops.iter().enumerate() {
        if fail_after == Some(i) {
            return Err(ApplyError::Simulated(i));
        }
        match op {
            Op::Write { path, .. } => {
                let dst = root.join(path);
                if let Some(parent) = dst.parent() {
                    io(fs::create_dir_all(parent), || {
                        format!("creating {}", parent.display())
                    })?;
                }
                io(fs::rename(staging.join(i.to_string()), &dst), || {
                    format!("installing {}", dst.display())
                })?;
                outcome.written += 1;
            }
            Op::Delete { path, .. } => {
                let p = root.join(path);
                match fs::remove_file(&p) {
                    Ok(()) => outcome.deleted += 1,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(ApplyError::Io {
                            context: format!("removing {}", p.display()),
                            source: e,
                        })
                    }
                }
            }
            Op::RemoveDirIfEmpty { path } => {
                let _ = fs::remove_dir(root.join(path));
            }
        }
    }

    // 7. Done
    let _ = fs::remove_dir_all(&staging);
    io(fs::remove_file(&journal_path), || {
        format!("removing {}", journal_path.display())
    })?;
    sync_dir(root);
    Ok(outcome)
}

/// What [`recover`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Recovery {
    pub restored: usize,
    pub removed: usize,
}

/// Rolls back an interrupted run. Returns `None` when there was nothing to recover.
pub fn recover(root: &Path) -> Result<Option<Recovery>, ApplyError> {
    let journal_path = root.join(JOURNAL);
    let bytes = match fs::read(&journal_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(ApplyError::Io {
                context: format!("reading {}", journal_path.display()),
                source: e,
            })
        }
    };
    let journal: Journal = serde_json::from_slice(&bytes).map_err(|e| ApplyError::Io {
        context: format!("parsing {}", journal_path.display()),
        source: std::io::Error::other(e),
    })?;
    let mut rec = Recovery::default();
    for entry in &journal.files {
        if !is_safe_relative(&entry.path) {
            continue;
        }
        let dst = root.join(&entry.path);
        if entry.existed {
            let src = journal.backup_dir.join(&entry.path);
            if let Some(parent) = dst.parent() {
                io(fs::create_dir_all(parent), || {
                    format!("creating {}", parent.display())
                })?;
            }
            io(fs::copy(&src, &dst), || {
                format!("restoring {}", dst.display())
            })?;
            rec.restored += 1;
        } else if dst.exists() {
            io(fs::remove_file(&dst), || {
                format!("removing {}", dst.display())
            })?;
            rec.removed += 1;
        }
    }
    let mut dirs = journal.created_dirs.clone();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.len()));
    for d in dirs {
        let _ = fs::remove_dir(root.join(d));
    }
    let _ = fs::remove_dir_all(root.join(STAGING));
    io(fs::remove_file(&journal_path), || {
        format!("removing {}", journal_path.display())
    })?;
    sync_dir(root);
    Ok(Some(rec))
}

fn missing_parents(root: &Path, plan: &Plan) -> Vec<String> {
    let mut out = Vec::new();
    for op in &plan.ops {
        let Op::Write { path, .. } = op else { continue };
        let mut p = Path::new(path).parent();
        while let Some(dir) = p {
            if dir.as_os_str().is_empty() {
                break;
            }
            let s = dir.to_string_lossy().into_owned();
            if !root.join(dir).exists() && !out.contains(&s) {
                out.push(s);
            }
            p = dir.parent();
        }
    }
    out
}

fn write_synced(path: &Path, content: &[u8], mode: u32) -> Result<(), ApplyError> {
    let mut f = io(fs::File::create(path), || {
        format!("creating {}", path.display())
    })?;
    io(f.write_all(content), || {
        format!("writing {}", path.display())
    })?;
    io(f.sync_all(), || format!("syncing {}", path.display()))?;
    set_mode(path, mode)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), ApplyError> {
    use std::os::unix::fs::PermissionsExt;
    io(
        fs::set_permissions(path, fs::Permissions::from_mode(mode)),
        || format!("setting mode on {}", path.display()),
    )
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> Result<(), ApplyError> {
    Ok(())
}

fn sync_dir(dir: &Path) {
    if let Ok(f) = fs::File::open(dir) {
        let _ = f.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::plan::{sha256_hex, Expect};

    fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            for e in fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(base, &p, out);
                } else {
                    out.push((
                        p.strip_prefix(base).unwrap().to_string_lossy().into_owned(),
                        fs::read(&p).unwrap(),
                    ));
                }
            }
        }
        walk(root, root, &mut out);
        out.sort();
        out
    }

    fn write_op(root: &Path, path: &str, content: &str) -> Op {
        Op::Write {
            path: path.into(),
            content: content.as_bytes().to_vec(),
            mode: 0o644,
            expect: current(root, path),
        }
    }

    fn setup() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        fs::create_dir_all(root.join("managed-settings.d")).unwrap();
        fs::write(root.join("CLAUDE.md"), "Company rule A\n").unwrap();
        fs::write(root.join("managed-settings.d/handrail-old.json"), "{}").unwrap();
        (t, root)
    }

    fn sample_plan(root: &Path) -> Plan {
        Plan {
            ops: vec![
                write_op(
                    root,
                    "managed-settings.d/handrail-privacy.json",
                    r#"{"a": 1}"#,
                ),
                write_op(
                    root,
                    "handrail/packs/privacy/hooks/guard.sh",
                    "#!/bin/sh\nexit 0\n",
                ),
                write_op(
                    root,
                    "CLAUDE.md",
                    "Company rule A\n\n<!-- handrail:begin -->\nx\n<!-- handrail:end -->\n",
                ),
                Op::Delete {
                    path: "managed-settings.d/handrail-old.json".into(),
                    expect: current(root, "managed-settings.d/handrail-old.json"),
                },
            ],
        }
    }

    #[test]
    fn applies_a_plan() {
        let (t, root) = setup();
        let out = apply(&root, &sample_plan(&root), &t.path().join("backup")).unwrap();
        assert_eq!((out.written, out.deleted), (3, 1));
        assert!(root.join("handrail/packs/privacy/hooks/guard.sh").exists());
        assert!(!root.join("managed-settings.d/handrail-old.json").exists());
        assert!(!root.join(JOURNAL).exists() && !root.join(STAGING).exists());
        // the replaced file is in the backup
        assert_eq!(
            fs::read_to_string(t.path().join("backup/CLAUDE.md")).unwrap(),
            "Company rule A\n"
        );
    }

    #[test]
    fn a_crash_at_any_point_recovers_to_the_old_state() {
        let ops = sample_plan(&setup().1).ops.len();
        for fail_after in 0..ops {
            let (t, root) = setup();
            let before = snapshot(&root);
            let plan = sample_plan(&root);
            let err =
                apply_with_failpoint(&root, &plan, &t.path().join("backup"), Some(fail_after))
                    .unwrap_err();
            assert!(matches!(err, ApplyError::Simulated(_)));
            // a second run must refuse until recovered
            assert!(matches!(
                apply(&root, &plan, &t.path().join("backup2")),
                Err(ApplyError::Interrupted(_))
            ));
            recover(&root).unwrap().expect("there was a journal");
            assert_eq!(
                snapshot(&root),
                before,
                "state after crash at op {fail_after} + recovery"
            );
        }
    }

    #[test]
    fn refuses_when_a_file_changed_since_planning() {
        let (t, root) = setup();
        let plan = sample_plan(&root);
        fs::write(root.join("CLAUDE.md"), "someone edited this\n").unwrap();
        let before = snapshot(&root);
        let err = apply(&root, &plan, &t.path().join("backup")).unwrap_err();
        assert!(
            matches!(err, ApplyError::Changed { ref path } if path == "CLAUDE.md"),
            "{err}"
        );
        assert_eq!(snapshot(&root), before, "nothing may change");
    }

    #[test]
    fn refuses_invalid_json_before_touching_anything() {
        let (t, root) = setup();
        let before = snapshot(&root);
        let plan = Plan {
            ops: vec![write_op(
                &root,
                "managed-settings.d/handrail-x.json",
                "{not json",
            )],
        };
        assert!(matches!(
            apply(&root, &plan, &t.path().join("b")),
            Err(ApplyError::InvalidJson { .. })
        ));
        assert_eq!(snapshot(&root), before);
    }

    #[test]
    fn refuses_paths_outside_the_root() {
        let (t, root) = setup();
        let plan = Plan {
            ops: vec![Op::Write {
                path: "../escape".into(),
                content: vec![],
                mode: 0o644,
                expect: Expect::Absent,
            }],
        };
        assert!(matches!(
            apply(&root, &plan, &t.path().join("b")),
            Err(ApplyError::UnsafePath { .. })
        ));
        assert!(!t.path().join("escape").exists());
    }

    #[test]
    fn recovery_removes_directories_the_run_created() {
        let (t, root) = setup();
        let plan = sample_plan(&root);
        let _ = apply_with_failpoint(&root, &plan, &t.path().join("backup"), Some(2));
        assert!(root.join("handrail/packs/privacy/hooks").exists());
        recover(&root).unwrap();
        assert!(
            !root.join("handrail").exists(),
            "handrail/ was created by the interrupted run"
        );
    }

    #[test]
    #[cfg(unix)]
    fn modes_are_applied() {
        use std::os::unix::fs::PermissionsExt;
        let (t, root) = setup();
        let plan = Plan {
            ops: vec![Op::Write {
                path: "hook.sh".into(),
                content: b"#!/bin/sh\n".to_vec(),
                mode: 0o755,
                expect: Expect::Absent,
            }],
        };
        apply(&root, &plan, &t.path().join("b")).unwrap();
        assert_eq!(
            fs::metadata(root.join("hook.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        let _ = sha256_hex(b"");
    }
}
