use super::*;
use crate::core::apply::apply;
use crate::core::catalog::DirSource;
use std::fs;

fn catalog() -> Catalog {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("catalog");
    Catalog::load(&DirSource(root)).unwrap_or_else(|p| panic!("{p:#?}"))
}

struct Env {
    _tmp: tempfile::TempDir,
    target: Target,
    backups: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let target = Target {
        managed_root: tmp.path().join("ClaudeCode"),
        user_dir: tmp.path().join("home/.claude"),
    };
    let backups = tmp.path().join("backups");
    Env {
        _tmp: tmp,
        target,
        backups,
    }
}

fn intent(packs: &[&str]) -> Intent {
    Intent {
        packs: packs.iter().map(|s| s.to_string()).collect(),
        local_rules: vec![],
        claude_version: Some("2.1.278 (Claude Code)".into()),
    }
}

fn profile(c: &Catalog, name: &str) -> Vec<String> {
    c.profiles[name].packs.clone()
}

fn run(e: &Env, c: &Catalog, i: &Intent) -> Plans {
    let plans = plan(c, &e.target, i, "test").unwrap();
    fs::create_dir_all(&e.target.managed_root).unwrap();
    fs::create_dir_all(&e.target.user_dir).unwrap();
    apply(
        &e.target.managed_root,
        &plans.enforced,
        &e.backups.join("e"),
    )
    .unwrap();
    apply(&e.target.user_dir, &plans.advisory, &e.backups.join("a")).unwrap();
    let _ = fs::remove_dir_all(&e.backups);
    plans
}

fn managed_files(e: &Env) -> Vec<String> {
    let d = e.target.managed_root.join("managed-settings.d");
    let mut v: Vec<String> = fs::read_dir(&d)
        .map(|rd| {
            rd.map(|x| x.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn baseline_installs_fragments_hooks_state_and_block() {
    let c = catalog();
    let e = env();
    let ids = profile(&c, "baseline");
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    run(&e, &c, &intent(&refs));
    assert_eq!(
        managed_files(&e),
        vec![
            "handrail-anti-bypass.json",
            "handrail-audit.json",
            "handrail-privacy.json"
        ]
    );
    let r = &e.target.managed_root;
    assert!(r.join("handrail/packs/privacy/hooks/guard.sh").exists());
    assert!(r.join("handrail/packs/audit/hooks/log.sh").exists());
    let md = fs::read_to_string(r.join("CLAUDE.md")).unwrap();
    assert!(
        md.starts_with(BEGIN)
            && md.contains("Keep data on this machine")
            && md.trim_end().ends_with(END)
    );
    let st = read_state(r).unwrap();
    assert_eq!(
        st.packs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["anti-bypass", "audit", "privacy"]
    );
}

#[test]
fn hook_paths_are_substituted_and_still_valid_json() {
    let c = catalog();
    let e = env();
    run(&e, &c, &intent(&["privacy"]));
    let raw = fs::read_to_string(
        e.target
            .managed_root
            .join("managed-settings.d/handrail-privacy.json"),
    )
    .unwrap();
    assert!(!raw.contains("@PACK_DIR@"));
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let cmd = v["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    let expected = e
        .target
        .managed_root
        .join("handrail/packs/privacy/hooks/guard.sh");
    assert_eq!(cmd, format!("\"{}\"", expected.display()));
}

#[test]
fn reapplying_the_same_intent_changes_nothing() {
    let c = catalog();
    let e = env();
    let i = intent(&["privacy", "audit"]);
    run(&e, &c, &i);
    let again = plan(&c, &e.target, &i, "test").unwrap();
    assert!(again.enforced.is_empty(), "{:#?}", again.enforced);
    assert!(again.advisory.is_empty());
}

#[test]
fn plans_are_deterministic() {
    let c = catalog();
    let e = env();
    let i = intent(&["privacy", "secrets", "sandbox"]);
    let a = plan(&c, &e.target, &i, "test").unwrap();
    let b = plan(&c, &e.target, &i, "test").unwrap();
    assert_eq!(a.enforced.hash(), b.enforced.hash());
}

#[test]
fn keeps_other_content_and_other_fragments() {
    let c = catalog();
    let e = env();
    let r = &e.target.managed_root;
    fs::create_dir_all(r.join("managed-settings.d")).unwrap();
    fs::write(r.join("CLAUDE.md"), "# Company rules\nRule A\n").unwrap();
    fs::write(
        r.join("managed-settings.d/00-org.json"),
        "{\"model\":\"opus\"}",
    )
    .unwrap();
    run(&e, &c, &intent(&["privacy"]));
    let md = fs::read_to_string(r.join("CLAUDE.md")).unwrap();
    assert!(
        md.starts_with("# Company rules\nRule A\n<!-- handrail:begin"),
        "{md}"
    );
    assert_eq!(
        fs::read_to_string(r.join("managed-settings.d/00-org.json")).unwrap(),
        "{\"model\":\"opus\"}"
    );
    // removing everything restores the company's file exactly and leaves their fragment
    run(&e, &c, &intent(&[]));
    assert_eq!(
        fs::read_to_string(r.join("CLAUDE.md")).unwrap(),
        "# Company rules\nRule A\n"
    );
    assert_eq!(managed_files(&e), vec!["00-org.json"]);
}

#[test]
fn switching_profiles_adds_and_removes_exactly() {
    let c = catalog();
    let e = env();
    let strict = profile(&c, "strict");
    let refs: Vec<&str> = strict.iter().map(String::as_str).collect();
    run(&e, &c, &intent(&refs));
    assert_eq!(managed_files(&e).len(), 6);
    run(&e, &c, &intent(&["privacy"]));
    assert_eq!(managed_files(&e), vec!["handrail-privacy.json"]);
    assert!(
        !e.target.managed_root.join("handrail/packs/audit").exists(),
        "audit's hook directory is pruned"
    );
    let md = fs::read_to_string(e.target.managed_root.join("CLAUDE.md")).unwrap();
    assert!(!md.contains("(secrets)"));
}

#[test]
fn removing_everything_leaves_nothing_of_ours() {
    let c = catalog();
    let e = env();
    let ids = profile(&c, "paranoid");
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut i = intent(&refs);
    i.local_rules = vec!["Reply in English".into()];
    run(&e, &c, &i);
    run(&e, &c, &intent(&[]));
    let left: Vec<String> = fs::read_dir(&e.target.managed_root)
        .unwrap()
        .map(|x| x.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}

#[test]
fn local_rules_land_in_the_block() {
    let c = catalog();
    let e = env();
    let mut i = intent(&[]);
    i.local_rules = vec!["Reply in English".into()];
    run(&e, &c, &i);
    let md = fs::read_to_string(e.target.managed_root.join("CLAUDE.md")).unwrap();
    assert!(
        md.contains("### Local rules\n\n- Reply in English\n"),
        "{md}"
    );
}

#[test]
fn packs_newer_than_the_installed_claude_code_are_skipped_with_a_reason() {
    let c = catalog();
    let e = env();
    let mut i = intent(&["privacy", "audit"]);
    i.claude_version = Some("2.1.100".into());
    let p = plan(&c, &e.target, &i, "test").unwrap();
    assert_eq!(p.enforced_packs, vec!["audit"]);
    assert_eq!(p.skipped.len(), 1);
    assert!(p.skipped[0].reason.contains("2.1.242"), "{:?}", p.skipped);
}

#[test]
fn unknown_packs_are_an_error() {
    let c = catalog();
    let e = env();
    assert!(matches!(
        plan(&c, &e.target, &intent(&["nope"]), "test"),
        Err(PlanError::UnknownPack(_))
    ));
}

#[test]
fn advisory_packs_go_to_the_users_rules_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let cat = tmp.path().join("catalog/packs/tone");
    fs::create_dir_all(&cat).unwrap();
    fs::write(
        cat.join("pack.toml"),
        "id = \"tone\"\nversion = \"1.0.0\"\ncategory = \"communication\"\ntier = \"advisory\"\n\
         title = \"t\"\nsummary = \"s\"\nlimits = \"l\"\n\n[targets.claude-code]\nenforcement = \"advisory\"\n",
    )
    .unwrap();
    fs::write(cat.join("rules.md"), "### Tone\n- Be brief.\n").unwrap();
    let c = Catalog::load(&DirSource(tmp.path().join("catalog"))).unwrap();
    let e = env();
    run(&e, &c, &intent(&["tone"]));
    assert_eq!(
        fs::read_to_string(e.target.user_dir.join("rules/handrail-tone.md")).unwrap(),
        "### Tone\n- Be brief.\n"
    );
    assert!(
        !e.target.managed_root.join("CLAUDE.md").exists(),
        "advisory packs never touch the managed directory"
    );
    run(&e, &c, &intent(&[]));
    assert!(!e.target.user_dir.join("rules/handrail-tone.md").exists());
}

#[test]
fn strip_block_keeps_everything_else_byte_for_byte() {
    let s = "a\n\n<!-- handrail:begin x -->\nours\n<!-- handrail:end -->\nb\n";
    assert_eq!(strip_block(s), "a\n\nb\n");
    assert_eq!(strip_block("no block\n"), "no block\n");
}

/// Each pack's hook test vectors (`tests/*.cases`), run by the same code as `handrail check`.
#[test]
fn hook_test_vectors() {
    let packs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("catalog/packs");
    let v = crate::check::hook_vectors(&packs);
    let failed: Vec<String> = v
        .iter()
        .filter(|x| !x.passed())
        .map(|x| format!("{}: {}", x.hook.display(), x.name))
        .collect();
    assert!(failed.is_empty(), "{failed:#?}");
    assert!(v.len() >= 16, "only {} vectors ran", v.len());
}
