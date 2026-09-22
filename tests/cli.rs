//! End-to-end tests of the `handrail` binary. Every run uses temporary directories for
//! the managed policy directory and ~/.claude: no root, nothing on the system is touched.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Env {
    tmp: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("ClaudeCode")).unwrap();
        Env { tmp }
    }
    fn root(&self) -> PathBuf {
        self.tmp.path().join("ClaudeCode")
    }
    fn user(&self) -> PathBuf {
        self.tmp.path().join("home/.claude")
    }
    fn run(&self, args: &[&str]) -> Output {
        self.run_with_version(args, "2.1.278 (Claude Code)")
    }
    fn run_with_version(&self, args: &[&str], version: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_handrail"))
            .args(args)
            .arg("--managed-root")
            .arg(self.root())
            .arg("--user-dir")
            .arg(self.user())
            .arg("--claude-version")
            .arg(version)
            .env("HOME", self.tmp.path().join("home"))
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let o = self.run(args);
        assert!(
            o.status.success(),
            "handrail {args:?} failed:\n{}{}",
            out(&o),
            err(&o)
        );
        out(&o)
    }
    fn fragments(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(self.root().join("managed-settings.d"))
            .map(|rd| {
                rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn list_shows_every_pack_with_its_enforcement() {
    let e = Env::new();
    let s = e.ok(&["list"]);
    for id in [
        "privacy",
        "anti-bypass",
        "secrets",
        "destructive",
        "supply-chain",
        "audit",
        "retention",
        "sandbox",
    ] {
        assert!(s.contains(id), "{id} missing from:\n{s}");
    }
    assert!(
        s.contains("partial"),
        "secrets and destructive are partial:\n{s}"
    );
}

#[test]
fn dry_run_writes_nothing() {
    let e = Env::new();
    let s = e.ok(&["use", "baseline", "--dry-run"]);
    assert!(
        s.contains("[dry run]") && s.contains("managed-settings.d/handrail-privacy.json"),
        "{s}"
    );
    assert_eq!(fs::read_dir(e.root()).unwrap().count(), 0);
}

#[test]
fn without_a_terminal_it_refuses_to_proceed_unconfirmed() {
    let e = Env::new();
    let o = e.run(&["use", "baseline"]);
    assert!(!o.status.success());
    assert!(err(&o).contains("--yes"), "{}", err(&o));
    assert_eq!(fs::read_dir(e.root()).unwrap().count(), 0);
}

#[test]
fn install_is_idempotent_and_keeps_existing_content() {
    let e = Env::new();
    fs::write(e.root().join("CLAUDE.md"), "# Company rules\nRule A\n").unwrap();
    e.ok(&["use", "baseline", "--yes"]);
    assert_eq!(
        e.fragments(),
        vec![
            "handrail-anti-bypass.json",
            "handrail-audit.json",
            "handrail-privacy.json"
        ]
    );
    assert!(e
        .ok(&["use", "baseline", "--yes"])
        .contains("Nothing to change."));
    let md = fs::read_to_string(e.root().join("CLAUDE.md")).unwrap();
    assert!(
        md.starts_with("# Company rules\nRule A\n<!-- handrail:begin"),
        "{md}"
    );
}

#[test]
fn enable_disable_and_profiles() {
    let e = Env::new();
    e.ok(&["use", "baseline", "--yes"]);
    e.ok(&["enable", "secrets", "--yes"]);
    assert!(e.fragments().contains(&"handrail-secrets.json".to_string()));
    e.ok(&["disable", "audit", "--yes"]);
    assert!(!e.fragments().contains(&"handrail-audit.json".to_string()));
    assert!(!e.root().join("handrail/packs/audit").exists());
    e.ok(&["use", "strict", "--yes"]);
    assert_eq!(e.fragments().len(), 6);
    let status = e.ok(&["status"]);
    assert!(
        status.contains("+ secrets") && status.contains("+ retention"),
        "{status}"
    );
}

#[test]
fn unknown_names_are_errors() {
    let e = Env::new();
    assert!(!e.run(&["enable", "nope", "--yes"]).status.success());
    assert!(!e.run(&["use", "nope", "--yes"]).status.success());
    assert!(!e.run(&["show", "nope"]).status.success());
}

#[test]
fn local_rules_and_rollback() {
    let e = Env::new();
    e.ok(&["use", "baseline", "--yes"]);
    e.ok(&["rule", "add", "Reply in English", "--yes"]);
    assert!(e.ok(&["rule", "list"]).contains("1. Reply in English"));
    assert!(fs::read_to_string(e.root().join("CLAUDE.md"))
        .unwrap()
        .contains("- Reply in English"));
    e.ok(&["rollback", "--yes"]);
    assert!(e.ok(&["rule", "list"]).contains("No local rules"));
    assert_eq!(
        e.fragments().len(),
        3,
        "rollback keeps the packs that were there before the rule"
    );
}

#[test]
fn doctor_detects_tampering() {
    let e = Env::new();
    e.ok(&["use", "baseline", "--yes"]);
    fs::write(
        e.root().join("managed-settings.d/handrail-privacy.json"),
        "{}",
    )
    .unwrap();
    let s = out(&e.run(&["doctor"]));
    assert!(
        s.contains("differs from what Handrail wrote") && s.contains("handrail-privacy.json"),
        "{s}"
    );
    // re-applying repairs it
    e.ok(&["use", "baseline", "--yes"]);
    assert!(!out(&e.run(&["doctor"])).contains("differs from what Handrail wrote"));
}

#[test]
fn disable_all_leaves_nothing_of_ours() {
    let e = Env::new();
    fs::write(e.root().join("CLAUDE.md"), "# Company rules\nRule A\n").unwrap();
    e.ok(&["use", "paranoid", "--yes"]);
    e.ok(&["rule", "add", "x", "--yes"]);
    e.ok(&["enable", "secrets", "--yes"]); // more backups
    e.ok(&["disable", "--all", "--yes"]);
    let left: Vec<String> = fs::read_dir(e.root())
        .unwrap()
        .map(|x| x.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, vec!["CLAUDE.md"]);
    assert_eq!(
        fs::read_to_string(e.root().join("CLAUDE.md")).unwrap(),
        "# Company rules\nRule A\n"
    );
}

#[test]
fn disable_all_on_a_directory_we_created_removes_it() {
    let e = Env::new();
    fs::remove_dir(e.root()).unwrap();
    e.ok(&["use", "baseline", "--yes"]);
    e.ok(&["disable", "--all", "--yes"]);
    assert!(!e.root().exists());
}

#[test]
fn packs_needing_a_newer_claude_code_are_skipped_and_explained() {
    let e = Env::new();
    let o = e.run_with_version(&["enable", "privacy", "audit", "--yes"], "2.1.100");
    let s = out(&o);
    assert!(o.status.success());
    assert!(
        s.contains("skipped privacy") && s.contains("2.1.242"),
        "{s}"
    );
    assert_eq!(e.fragments(), vec!["handrail-audit.json"]);
}

// ── Static checks on shipped files ─────────────────────────────────────

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            files_under(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// sh swallows the leading bytes of a multibyte character into a variable name:
/// "$VERSION）" reads a variable named VERSION\xef\xbc and aborts under `set -u`.
/// This stopped the M0 installer halfway once. Hooks are shell scripts, so check them.
#[test]
fn hook_scripts_never_put_non_ascii_right_after_a_variable() {
    let mut files = vec![];
    files_under(&repo().join("catalog"), &mut files);
    for f in files
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "sh"))
    {
        let s = fs::read_to_string(f).unwrap();
        for (n, line) in s.lines().enumerate() {
            let b = line.as_bytes();
            for i in 0..b.len() {
                if b[i] != b'$' {
                    continue;
                }
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                assert!(
                    !(j > i + 1 && j < b.len() && b[j] >= 0x80),
                    "{}:{}: use ${{VAR}} before non-ASCII text",
                    f.display(),
                    n + 1
                );
            }
        }
    }
}

/// The project defaults to English: no CJK text in anything we ship.
#[test]
fn shipped_text_is_english() {
    let mut files = vec![];
    files_under(&repo().join("catalog"), &mut files);
    files_under(&repo().join("src"), &mut files);
    files_under(&repo().join("tests"), &mut files);
    files_under(&repo().join("docs"), &mut files);
    files.push(repo().join("README.md"));
    for f in files
        .iter()
        .filter(|f| !f.components().any(|c| c.as_os_str() == "target"))
    {
        let Ok(s) = fs::read_to_string(f) else {
            continue;
        };
        if let Some((n, _)) = s
            .lines()
            .enumerate()
            .find(|(_, l)| l.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)))
        {
            panic!("{}:{}: CJK text in a shipped file", f.display(), n + 1);
        }
    }
}

#[test]
fn statusline_reports_profile_packs_drift_and_off() {
    let e = Env::new();
    assert_eq!(e.ok(&["statusline"]).trim(), "handrail: off");
    e.ok(&["use", "baseline", "--yes"]);
    assert_eq!(e.ok(&["statusline"]).trim(), "handrail: baseline ✓");
    e.ok(&["enable", "secrets", "--yes"]);
    assert_eq!(e.ok(&["statusline"]).trim(), "handrail: 4 packs ✓");
    fs::write(
        e.root().join("managed-settings.d/handrail-privacy.json"),
        "{}",
    )
    .unwrap();
    assert_eq!(e.ok(&["statusline"]).trim(), "handrail: 4 packs drift ⚠");
}
