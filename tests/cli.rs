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
    assert!(e.ok(&["rule", "list"]).contains("  Reply in English"));
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
        s.contains("differ from what Handrail wrote") && s.contains("handrail-privacy.json"),
        "{s}"
    );
    // re-applying repairs it
    e.ok(&["use", "baseline", "--yes"]);
    assert!(!out(&e.run(&["doctor"])).contains("differ from what Handrail wrote"));
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

#[test]
fn check_passes_on_the_shipped_catalog_and_fails_on_a_broken_pack() {
    let e = Env::new();
    let ok = e.run(&["check", repo().join("catalog").to_str().unwrap()]);
    assert!(ok.status.success(), "{}{}", out(&ok), err(&ok));
    assert!(out(&ok).contains("All checks passed"));
    // a broken copy: invalid settings JSON and a hook that fails its own vector
    let broken = e.tmp.path().join("broken");
    fs::create_dir_all(broken.join("packs/x/claude-code/hooks")).unwrap();
    fs::create_dir_all(broken.join("packs/x/tests")).unwrap();
    fs::write(broken.join("packs/x/pack.toml"), "id = \"x\"\nversion = \"1.0.0\"\ncategory = \"security\"\ntier = \"enforced\"\ntitle = \"t\"\nsummary = \"s\"\nlimits = \"l\"\n[targets.claude-code]\nenforcement = \"enforced\"\nsettings = \"claude-code/settings.json\"\nhooks = [\"claude-code/hooks/g.sh\"]\n").unwrap();
    fs::write(broken.join("packs/x/rules.md"), "### x\n").unwrap();
    fs::write(
        broken.join("packs/x/claude-code/settings.json"),
        "{not json",
    )
    .unwrap();
    fs::write(
        broken.join("packs/x/claude-code/hooks/g.sh"),
        "#!/bin/sh\nV=1\necho \"$V\u{ff09}\"\nexit 0\n",
    )
    .unwrap();
    fs::write(
        broken.join("packs/x/tests/g.cases"),
        "should block\t2\t{}\n",
    )
    .unwrap();
    let bad = e.run(&["check", broken.to_str().unwrap()]);
    assert!(!bad.status.success());
    let s = out(&bad);
    assert!(
        s.contains("invalid JSON") && s.contains("should block") && s.contains("use ${VAR}"),
        "{s}"
    );
}

/// Hooks are shell scripts: the same lint `handrail check` applies.
#[test]
fn hook_scripts_never_put_non_ascii_right_after_a_variable() {
    let problems = handrail_lint(&repo().join("catalog"));
    assert!(problems.is_empty(), "{problems:#?}");
}

fn handrail_lint(dir: &Path) -> Vec<String> {
    let o = Command::new(env!("CARGO_BIN_EXE_handrail"))
        .arg("check")
        .arg(dir)
        .output()
        .unwrap();
    out(&o)
        .lines()
        .filter(|l| l.contains("use ${VAR}"))
        .map(String::from)
        .collect()
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

#[test]
fn statusline_shows_a_newer_release_only_when_the_check_is_on() {
    let e = Env::new();
    e.ok(&["use", "baseline", "--yes"]);
    let cache = e.user().join("handrail/update-check.json");
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    let recent = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // off (the default): no hint, even if a newer version is cached
    fs::write(
        &cache,
        format!(r#"{{"enabled": false, "checked_at": {recent}, "latest": "9.9.9"}}"#),
    )
    .unwrap();
    assert_eq!(e.ok(&["statusline"]).trim(), "handrail: baseline ✓");
    // on, recent cache: shows the hint without touching the network
    fs::write(
        &cache,
        format!(r#"{{"enabled": true, "checked_at": {recent}, "latest": "9.9.9"}}"#),
    )
    .unwrap();
    assert_eq!(e.ok(&["statusline"]).trim(), "handrail: baseline ✓ ↑9.9.9");
    // on, but the cached release is not newer: no hint
    fs::write(
        &cache,
        format!(r#"{{"enabled": true, "checked_at": {recent}, "latest": "0.0.1"}}"#),
    )
    .unwrap();
    assert_eq!(e.ok(&["statusline"]).trim(), "handrail: baseline ✓");
}

/// A one-pack catalog in `dir`: a copy of the built-in `audit` pack under a new id.
fn external_catalog(dir: &Path, id: &str) {
    let src = repo().join("catalog/packs/audit");
    let dst = dir.join("packs").join(id);
    let mut files = Vec::new();
    files_under(&src, &mut files);
    for f in files {
        let rel = f.strip_prefix(&src).unwrap();
        let to = dst.join(rel);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        let mut bytes = fs::read(&f).unwrap();
        if rel == Path::new("pack.toml") {
            bytes = String::from_utf8(bytes)
                .unwrap()
                .replace("id = \"audit\"", &format!("id = \"{id}\""))
                .into_bytes();
        }
        fs::write(&to, bytes).unwrap();
    }
}

#[test]
fn external_packs_install_from_a_directory_and_survive_without_it() {
    let e = Env::new();
    let cat = e.tmp.path().join("my-packs");
    external_catalog(&cat, "my-audit");
    let c = cat.to_str().unwrap();

    assert!(e.ok(&["list", "--catalog", c]).contains("my-audit"));
    e.ok(&["enable", "my-audit", "--catalog", c, "-y"]);
    let copy = e.root().join("handrail/external/packs/my-audit");
    assert!(copy.join("pack.toml").is_file());
    let state = fs::read_to_string(e.root().join("handrail/state.json")).unwrap();
    assert!(state.contains(&format!(
        "\"origin\": \"{}\"",
        cat.canonicalize().unwrap().display()
    )));

    // The source directory can go away: status and disable use the installed copy.
    fs::remove_dir_all(&cat).unwrap();
    let s = e.ok(&["status"]);
    assert!(s.contains("my-audit"), "{s}");
    assert!(!s.contains('⚠'), "{s}");
    e.ok(&["disable", "my-audit", "-y"]);
    assert!(!e.root().join("handrail/external").exists());
}

#[test]
fn external_packs_install_from_a_git_repository() {
    let e = Env::new();
    let cat = e.tmp.path().join("repo");
    external_catalog(&cat, "team-audit");
    let git = |args: &[&str]| {
        let o = Command::new("git")
            .current_dir(&cat)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "git {args:?}: {}", err(&o));
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "-qm",
        "packs",
    ]);
    let url = format!("file://{}", cat.display());
    e.ok(&["enable", "team-audit", "--catalog", &url, "-y"]);
    let state = fs::read_to_string(e.root().join("handrail/state.json")).unwrap();
    assert!(state.contains(&url), "{state}");
}

#[test]
fn external_packs_cannot_reuse_a_built_in_id() {
    let e = Env::new();
    let cat = e.tmp.path().join("evil");
    external_catalog(&cat, "privacy");
    let o = e.run(&["list", "--catalog", cat.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(err(&o).contains("already a built-in pack"), "{}", err(&o));
}

/// Requests the fake GitHub received: (method, path, body).
type RequestLog = std::sync::Arc<std::sync::Mutex<Vec<(String, String, String)>>>;

/// A stand-in for api.github.com: answers the calls `handrail publish` makes, as a user
/// without push access to the catalog (so a fork is used), and records every request.
fn fake_github() -> (String, RequestLog) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut r = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("").to_string();
            let (mut len, mut auth) = (0usize, false);
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                let lower = h.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                if lower.starts_with("authorization: bearer test-token") {
                    auth = true;
                }
            }
            let mut body = vec![0; len];
            r.read_exact(&mut body).unwrap();
            let body = String::from_utf8(body).unwrap();
            let up = "/repos/martinx/handrail-packs";
            let fork = "/repos/alice/handrail-packs";
            let (status, reply) = match (method.as_str(), path.as_str()) {
                _ if !auth => (401, r#"{"message":"Bad credentials"}"#.to_string()),
                ("GET", "/user") => (200, r#"{"login":"alice"}"#.into()),
                ("GET", p) if p == up => {
                    (200, r#"{"default_branch":"main","permissions":{"push":false}}"#.into())
                }
                ("POST", p) if p == format!("{up}/forks") => {
                    (202, r#"{"full_name":"alice/handrail-packs"}"#.into())
                }
                ("GET", p) if p == fork => (200, "{}".into()),
                ("GET", p) if p == format!("{up}/git/ref/heads/main") => {
                    (200, r#"{"object":{"sha":"base"}}"#.into())
                }
                ("POST", p) if p == format!("{fork}/merge-upstream") => (200, "{}".into()),
                ("GET", p) if p == format!("{fork}/git/commits/base") => {
                    (200, r#"{"tree":{"sha":"tree0"}}"#.into())
                }
                ("GET", p) if p.starts_with(&format!("{fork}/git/trees/tree0")) => (
                    200,
                    r#"{"tree":[{"path":"packs/my-audit","type":"tree"},{"path":"packs/my-audit/old.md","type":"blob"},{"path":"packs/other/pack.toml","type":"blob"}]}"#.into(),
                ),
                ("POST", p) if p == format!("{fork}/git/blobs") => (201, r#"{"sha":"blob"}"#.into()),
                ("POST", p) if p == format!("{fork}/git/trees") => (201, r#"{"sha":"tree1"}"#.into()),
                ("POST", p) if p == format!("{fork}/git/commits") => (201, r#"{"sha":"c1"}"#.into()),
                ("POST", p) if p == format!("{fork}/git/refs") => (201, "{}".into()),
                ("POST", p) if p == format!("{up}/pulls") => {
                    (201, r#"{"html_url":"https://github.com/martinx/handrail-packs/pull/7"}"#.into())
                }
                _ => (404, r#"{"message":"Not Found"}"#.into()),
            };
            seen.lock().unwrap().push((method, path, body));
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    (base, log)
}

#[test]
fn publish_opens_a_pull_request_from_a_fork() {
    let e = Env::new();
    let cat = e.tmp.path().join("my-packs");
    external_catalog(&cat, "my-audit");
    let (api, log) = fake_github();
    let o = Command::new(env!("CARGO_BIN_EXE_handrail"))
        .args([
            "publish",
            "my-audit",
            "--catalog",
            cat.to_str().unwrap(),
            "-y",
        ])
        .arg("--managed-root")
        .arg(e.root())
        .arg("--user-dir")
        .arg(e.user())
        .args(["--claude-version", "2.1.278 (Claude Code)"])
        .env("HOME", e.tmp.path().join("home"))
        .env("GITHUB_TOKEN", "test-token")
        .env_remove("GH_TOKEN")
        .env("HANDRAIL_GITHUB_API", &api)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert!(out(&o).contains("https://github.com/martinx/handrail-packs/pull/7"));

    let log = log.lock().unwrap();
    let body = |path: &str| -> serde_json::Value {
        let (_, _, b) = log
            .iter()
            .find(|(m, p, _)| m == "POST" && p.ends_with(path))
            .unwrap();
        serde_json::from_str(b).unwrap()
    };
    // Every file of the pack, and the file an earlier version had, removed; nothing else
    let tree = body("/git/trees");
    let entries = tree["tree"].as_array().unwrap();
    assert!(entries
        .iter()
        .any(|e| e["path"] == "packs/my-audit/old.md" && e["sha"].is_null()));
    assert!(entries
        .iter()
        .any(|e| e["path"] == "packs/my-audit/pack.toml"));
    assert!(entries
        .iter()
        .any(|e| e["path"] == "packs/my-audit/claude-code/hooks/log.sh" && e["mode"] == "100755"));
    assert!(entries
        .iter()
        .all(|e| e["path"].as_str().unwrap().starts_with("packs/my-audit/")));
    let pr = body("/pulls");
    assert_eq!(pr["head"], "alice:pack/my-audit-1.0.0");
    assert_eq!(pr["base"], "main");
    assert_eq!(pr["title"], "Update my-audit v1.0.0");
    assert!(pr["body"].as_str().unwrap().contains("handrail check"));
}

#[test]
fn publish_refuses_built_in_packs() {
    let e = Env::new();
    let o = e.run(&["publish", "privacy", "--dry-run"]);
    assert!(!o.status.success());
    assert!(err(&o).contains("built-in pack"), "{}", err(&o));
}

#[test]
fn rules_are_referred_to_by_id() {
    let e = Env::new();
    e.ok(&["rule", "add", "Reply in English", "--yes"]);
    e.ok(&["rule", "add", "Small commits", "--id", "commits", "--yes"]);
    let list = e.ok(&["rule", "list"]);
    let generated = list
        .lines()
        .find(|l| l.contains("Reply in English"))
        .and_then(|l| l.split_whitespace().next())
        .unwrap()
        .to_string();
    assert!(
        generated.starts_with("rule-") && generated.len() == 9,
        "{list}"
    );
    assert!(
        list.lines()
            .any(|l| l.split_whitespace().collect::<Vec<_>>() == ["commits", "Small", "commits"]),
        "{list}"
    );

    // A chosen id must be unused and well-formed; numbers are no longer references
    assert!(!e
        .run(&["rule", "add", "x", "--id", "commits", "--yes"])
        .status
        .success());
    assert!(!e
        .run(&["rule", "add", "x", "--id", "No Spaces", "--yes"])
        .status
        .success());
    assert!(!e.run(&["rule", "remove", "1", "--yes"]).status.success());

    // Editing keeps the id
    e.ok(&["rule", "edit", &generated, "Reply in Chinese", "--yes"]);
    let list = e.ok(&["rule", "list"]);
    assert!(
        list.contains(&format!("{generated}  Reply in Chinese")),
        "{list}"
    );
    let md = fs::read_to_string(e.root().join("CLAUDE.md")).unwrap();
    assert!(
        md.contains("- Reply in Chinese") && !md.contains("Reply in English"),
        "{md}"
    );

    e.ok(&["rule", "remove", &generated, "commits", "--yes"]);
    assert!(e.ok(&["rule", "list"]).contains("No local rules"));
}

#[test]
fn rules_from_older_state_get_ids_without_changing_the_installed_files() {
    let e = Env::new();
    e.ok(&["rule", "add", "Reply in English", "--yes"]);
    // Rewrite the state the way 0.2.0 and earlier stored rules: plain strings
    let path = e.root().join("handrail/state.json");
    let mut state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    state["local_rules"] = serde_json::json!(["Reply in English"]);
    fs::write(&path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
    let before = fs::read_to_string(e.root().join("CLAUDE.md")).unwrap();

    let id = e
        .ok(&["rule", "list"])
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    assert!(id.starts_with("rule-"));
    assert_eq!(
        e.ok(&["rule", "list"]).split_whitespace().next().unwrap(),
        id,
        "stable across reads"
    );
    assert!(!e.ok(&["status"]).contains('⚠'));
    e.ok(&["rule", "remove", &id, "--yes"]);
    assert!(!fs::read_to_string(e.root().join("CLAUDE.md"))
        .unwrap_or_default()
        .contains("Reply in English"));
    let _ = before;
}

/// Rewrites the enforced state as if another Handrail version had written it.
fn edit_state(e: &Env, f: impl FnOnce(&mut serde_json::Value)) {
    let path = e.root().join("handrail/state.json");
    let mut state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    f(&mut state);
    fs::write(&path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
}

#[test]
fn upgrading_handrail_or_a_pack_without_file_changes_asks_for_nothing() {
    let e = Env::new();
    e.ok(&["use", "baseline", "--yes"]);
    edit_state(&e, |s| {
        s["handrail_version"] = "0.0.1".into();
        s["packs"][0]["version"] = "0.9.0".into();
    });
    let status = e.ok(&["status"]);
    assert!(
        !status.contains("Re-apply") && !status.contains("re-apply"),
        "{status}"
    );
    assert_eq!(e.ok(&["statusline"]).trim(), "handrail: baseline ✓");
}

#[test]
fn a_newer_pack_version_is_named_in_status_and_the_status_line() {
    let e = Env::new();
    e.ok(&["use", "baseline", "--yes"]);
    edit_state(&e, |s| {
        s["handrail_version"] = "0.0.1".into();
        for p in s["packs"].as_array_mut().unwrap() {
            if p["id"] == "privacy" {
                p["version"] = "0.9.0".into();
            }
        }
    });
    // What version 0.9.0 of the pack installed
    fs::write(
        e.root().join("managed-settings.d/handrail-privacy.json"),
        "{}",
    )
    .unwrap();
    let status = e.ok(&["status"]);
    assert!(
        status.contains("Pack updates available: privacy 0.9.0 → 1.0.0"),
        "{status}"
    );
    assert!(e.ok(&["statusline"]).contains("re-apply"));
}

#[test]
fn a_version_in_the_old_block_title_is_not_a_reason_to_re_apply() {
    let e = Env::new();
    e.ok(&["use", "baseline", "--yes"]);
    let path = e.root().join("CLAUDE.md");
    let md = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        md.replace(
            "# Local policy (Handrail)",
            "# Local policy (Handrail 0.2.0)",
        ),
    )
    .unwrap();
    edit_state(&e, |s| s["handrail_version"] = "0.2.0".into());
    assert_eq!(e.ok(&["statusline"]).trim(), "handrail: baseline ✓");
    // Real edits to the block are still caught
    fs::write(
        &path,
        md.replace("Put nothing on claude.ai", "Anything goes"),
    )
    .unwrap();
    assert!(e.ok(&["statusline"]).contains("drift ⚠"));
    assert!(e
        .ok(&["status"])
        .contains("differ from what Handrail wrote"));
}
