//! Claude Code target: turns "these packs, these local rules" into an exact file plan.
//!
//! ## Where things go
//!
//! | Tier | Location | Owner |
//! |---|---|---|
//! | enforced | managed policy directory: `managed-settings.d/handrail-<id>.json`, hooks under `handrail/packs/<id>/hooks/`, a marked block in `CLAUDE.md` | root |
//! | advisory | `~/.claude/rules/handrail-<id>.md` (rules without `paths` frontmatter load at launch, in every project) | the user |
//!
//! Managed settings take precedence over user, project and command-line settings; the
//! managed `CLAUDE.md` is loaded in every session and cannot be excluded. Fragments in
//! `managed-settings.d/` merge in name order: lists are unioned, objects merge by key,
//! and a later scalar wins — which is why the catalog rejects scalar conflicts.
//!
//! ## Determinism
//!
//! A plan depends only on the catalog, the intent and the files on disk. No timestamps,
//! no randomness. The privileged step recomputes the plan from the intent and compares
//! its hash with the one the user reviewed.

use crate::core::catalog::{Catalog, Origin, Pack, Tier};
use crate::core::plan::{current, Expect, Op, Plan};
use crate::core::rule::LocalRule;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const TARGET: &str = "claude-code";
pub const MARK: &str = "handrail";
const STATE: &str = "handrail/state.json";

/// Where Claude Code reads configuration on this machine.
#[derive(Debug, Clone)]
pub struct Target {
    /// The managed policy directory (root-owned).
    pub managed_root: PathBuf,
    /// The user's `~/.claude` directory.
    pub user_dir: PathBuf,
}

impl Target {
    /// The system locations for this OS, as documented by Claude Code.
    pub fn system(home: &Path) -> Result<Target, String> {
        let managed_root = if cfg!(target_os = "macos") {
            PathBuf::from("/Library/Application Support/ClaudeCode")
        } else if cfg!(target_os = "linux") {
            PathBuf::from("/etc/claude-code")
        } else {
            return Err("this platform is not supported yet; on Windows, run inside WSL2".into());
        };
        Ok(Target {
            managed_root,
            user_dir: home.join(".claude"),
        })
    }
}

/// What the user wants: which packs, which local rules.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent {
    pub packs: BTreeSet<String>,
    pub local_rules: Vec<LocalRule>,
    /// Output of `claude --version`, used for `min_version` gating. `None` = unknown.
    pub claude_version: Option<String>,
}

/// What is installed, as recorded by the last successful apply.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub handrail_version: String,
    pub packs: Vec<InstalledPack>,
    #[serde(default)]
    pub local_rules: Vec<LocalRule>,
    /// Every file this tier owns, relative to its root. Uninstall removes exactly these.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPack {
    pub id: String,
    pub version: String,
    /// Where an external pack came from (directory or repository). Absent for built-in packs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

fn installed(p: &Pack) -> InstalledPack {
    InstalledPack {
        id: p.id().into(),
        version: p.manifest.version.clone(),
        origin: match &p.origin {
            Origin::Builtin => None,
            Origin::External(s) => Some(s.clone()),
        },
    }
}

/// Where the installed copy of external packs lives, relative to a tier's root.
pub const EXTERNAL: &str = "handrail/external";

/// The copy of an external pack's own files, installed alongside it. Everything later
/// (status, drift checks, removal, the privileged step of the next change) reads this
/// copy, which sits in a root-owned directory for enforced packs, instead of the author's
/// directory, which anything running as the user could change.
fn vendored(p: &Pack) -> Vec<(String, Vec<u8>, u32)> {
    if p.origin == Origin::Builtin {
        return vec![];
    }
    p.files
        .iter()
        .map(|(rel, bytes)| {
            let mode = if rel.ends_with(".sh") { 0o755 } else { 0o644 };
            (
                format!("{EXTERNAL}/packs/{}/{rel}", p.id()),
                bytes.clone(),
                mode,
            )
        })
        .collect()
}

pub fn read_state(root: &Path) -> Option<State> {
    serde_json::from_slice(&std::fs::read(root.join(STATE)).ok()?).ok()
}

/// A pack that was requested but left out, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct Plans {
    pub enforced: Plan,
    pub advisory: Plan,
    pub skipped: Vec<Skipped>,
    /// Packs that will be active on each tier after applying.
    pub enforced_packs: Vec<String>,
    pub advisory_packs: Vec<String>,
}

#[derive(Debug)]
pub enum PlanError {
    UnknownPack(String),
}

impl std::error::Error for PlanError {}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::UnknownPack(id) => write!(f, "no pack named \"{id}\" (see: handrail list)"),
        }
    }
}

/// Computes both plans for `intent`.
pub fn plan(
    catalog: &Catalog,
    target: &Target,
    intent: &Intent,
    handrail_version: &str,
) -> Result<Plans, PlanError> {
    let mut plans = Plans::default();
    let mut enforced: Vec<&Pack> = Vec::new();
    let mut advisory: Vec<&Pack> = Vec::new();
    for id in &intent.packs {
        let pack = catalog
            .packs
            .get(id)
            .ok_or_else(|| PlanError::UnknownPack(id.clone()))?;
        let Some(files) = pack.targets.get(TARGET) else {
            plans.skipped.push(Skipped {
                id: id.clone(),
                reason: "has no Claude Code implementation".into(),
            });
            continue;
        };
        if let (Some(required), Some(installed)) = (&files.spec.min_version, &intent.claude_version)
        {
            if crate::core::version::at_least(installed, required) == Some(false) {
                plans.skipped.push(Skipped {
                    id: id.clone(),
                    reason: format!(
                        "needs Claude Code {required} or later (installed: {installed})"
                    ),
                });
                continue;
            }
        }
        match pack.manifest.tier {
            Tier::Enforced => enforced.push(pack),
            Tier::Advisory => advisory.push(pack),
        }
    }
    plans.enforced_packs = enforced.iter().map(|p| p.id().to_string()).collect();
    plans.advisory_packs = advisory.iter().map(|p| p.id().to_string()).collect();
    plans.enforced = enforced_plan(
        &target.managed_root,
        &enforced,
        &intent.local_rules,
        handrail_version,
    );
    plans.advisory = advisory_plan(&target.user_dir, &advisory, handrail_version);
    Ok(plans)
}

fn enforced_plan(root: &Path, packs: &[&Pack], local_rules: &[LocalRule], version: &str) -> Plan {
    let mut desired: Vec<(String, Vec<u8>, u32)> = Vec::new();
    for pack in packs {
        let files = &pack.targets[TARGET];
        let pack_dir = root.join(MARK).join("packs").join(pack.id());
        if let Some(settings) = &files.settings {
            let rendered = substitute(settings, "@PACK_DIR@", &pack_dir.to_string_lossy());
            let mut bytes = serde_json::to_vec_pretty(&rendered).expect("settings serialise");
            bytes.push(b'\n');
            desired.push((
                format!("managed-settings.d/{MARK}-{}.json", pack.id()),
                bytes,
                0o644,
            ));
        }
        for hook in &files.hooks {
            desired.push((
                format!("{MARK}/packs/{}/hooks/{}", pack.id(), hook.name),
                hook.content.clone(),
                0o755,
            ));
        }
        desired.extend(vendored(pack));
    }
    let active = !packs.is_empty() || !local_rules.is_empty();
    let state = State {
        handrail_version: version.to_string(),
        packs: packs.iter().map(|p| installed(p)).collect(),
        local_rules: local_rules.to_vec(),
        files: desired.iter().map(|(p, _, _)| p.clone()).collect(),
    };

    let mut plan = Plan::default();
    let old_files: Vec<String> = read_state(root).map(|s| s.files).unwrap_or_default();

    // CLAUDE.md is shared with whoever else manages this machine: only our block changes
    let claude_md = root.join("CLAUDE.md");
    let existing = std::fs::read_to_string(&claude_md).ok();
    let kept = existing.as_deref().map(strip_block).unwrap_or_default();
    let new_md = if active {
        // Append without a separator: removing the block later must give back the
        // original bytes exactly, and a separator we added would be left behind.
        // (Only a file with no final newline changes: it gains one.)
        let mut s = kept.clone();
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(&render_block(packs, local_rules, version));
        Some(s)
    } else if kept.trim().is_empty() {
        None
    } else {
        Some(kept.clone())
    };
    match (&existing, &new_md) {
        (Some(old), Some(new)) if old == new => {}
        (_, Some(new)) => push_write(&mut plan, root, "CLAUDE.md", new.as_bytes().to_vec(), 0o644),
        (Some(_), None) => plan.ops.push(Op::Delete {
            path: "CLAUDE.md".into(),
            expect: current(root, "CLAUDE.md"),
        }),
        (None, None) => {}
    }

    for (path, bytes, mode) in &desired {
        push_write(&mut plan, root, path, bytes.clone(), *mode);
    }
    let wanted: BTreeSet<&String> = desired.iter().map(|(p, _, _)| p).collect();
    for old in &old_files {
        if !wanted.contains(old) && root.join(old).exists() {
            plan.ops.push(Op::Delete {
                path: old.clone(),
                expect: current(root, old),
            });
        }
    }
    if active {
        let mut bytes = serde_json::to_vec_pretty(&state).expect("state serialises");
        bytes.push(b'\n');
        push_write(&mut plan, root, STATE, bytes, 0o644);
    } else if root.join(STATE).exists() {
        plan.ops.push(Op::Delete {
            path: STATE.into(),
            expect: current(root, STATE),
        });
    }
    prune_dirs(&mut plan, root, &old_files);
    plan
}

fn advisory_plan(root: &Path, packs: &[&Pack], version: &str) -> Plan {
    let mut desired: Vec<(String, Vec<u8>, u32)> = Vec::new();
    for p in packs {
        desired.push((
            format!("rules/{MARK}-{}.md", p.id()),
            p.rules.clone().into_bytes(),
            0o644,
        ));
        desired.extend(vendored(p));
    }
    let state = State {
        handrail_version: version.to_string(),
        packs: packs.iter().map(|p| installed(p)).collect(),
        local_rules: vec![],
        files: desired.iter().map(|(p, _, _)| p.clone()).collect(),
    };
    let old_files = read_state(root).map(|s| s.files).unwrap_or_default();
    let mut plan = Plan::default();
    for (path, bytes, mode) in &desired {
        push_write(&mut plan, root, path, bytes.clone(), *mode);
    }
    for old in &old_files {
        if !desired.iter().any(|(p, _, _)| p == old) && root.join(old).exists() {
            plan.ops.push(Op::Delete {
                path: old.clone(),
                expect: current(root, old),
            });
        }
    }
    if !packs.is_empty() {
        let mut bytes = serde_json::to_vec_pretty(&state).expect("state serialises");
        bytes.push(b'\n');
        push_write(&mut plan, root, STATE, bytes, 0o644);
    } else if root.join(STATE).exists() {
        plan.ops.push(Op::Delete {
            path: STATE.into(),
            expect: current(root, STATE),
        });
    }
    prune_dirs(&mut plan, root, &old_files);
    plan
}

/// Adds a write only if the content or mode actually differs, so re-applying is a no-op.
fn push_write(plan: &mut Plan, root: &Path, path: &str, content: Vec<u8>, mode: u32) {
    let expect = current(root, path);
    if let Expect::Hash(h) = &expect {
        if *h == crate::core::plan::sha256_hex(&content) && mode_of(&root.join(path)) == Some(mode)
        {
            return;
        }
    }
    plan.ops.push(Op::Write {
        path: path.into(),
        content,
        mode,
        expect,
    });
}

#[cfg(unix)]
fn mode_of(p: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .ok()
        .map(|m| m.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn mode_of(_: &Path) -> Option<u32> {
    None
}

/// After deletions, remove directories we created if they end up empty (deepest first).
fn prune_dirs(plan: &mut Plan, _root: &Path, old_files: &[String]) {
    let deleting: BTreeSet<&str> = plan
        .ops
        .iter()
        .filter_map(|op| {
            if let Op::Delete { path, .. } = op {
                Some(path.as_str())
            } else {
                None
            }
        })
        .collect();
    if deleting.is_empty() {
        return;
    }
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    for f in old_files
        .iter()
        .map(String::as_str)
        .chain(deleting.iter().copied())
    {
        let mut p = Path::new(f).parent();
        while let Some(d) = p {
            if d.as_os_str().is_empty() {
                break;
            }
            dirs.insert(d.to_string_lossy().into_owned());
            p = d.parent();
        }
    }
    let mut dirs: Vec<String> = dirs.into_iter().collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.matches('/').count()));
    for d in dirs {
        plan.ops.push(Op::RemoveDirIfEmpty { path: d });
    }
}

/// Replaces a placeholder inside every string of a JSON value. Working on the value, not
/// the text, keeps the result valid JSON whatever the substituted path contains.
fn substitute(v: &serde_json::Value, from: &str, to: &str) -> serde_json::Value {
    match v {
        serde_json::Value::String(s) => serde_json::Value::String(s.replace(from, to)),
        serde_json::Value::Array(a) => {
            serde_json::Value::Array(a.iter().map(|x| substitute(x, from, to)).collect())
        }
        serde_json::Value::Object(o) => serde_json::Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), substitute(x, from, to)))
                .collect(),
        ),
        other => other.clone(),
    }
}

pub const BEGIN: &str = "<!-- handrail:begin";
pub const END: &str = "<!-- handrail:end -->";

/// Removes our block from a CLAUDE.md, keeping everything else byte for byte.
pub fn strip_block(s: &str) -> String {
    let mut out = String::new();
    let mut skip = false;
    for line in s.split_inclusive('\n') {
        if line.starts_with(BEGIN) {
            skip = true;
        }
        if !skip {
            out.push_str(line);
        }
        if skip && line.trim_end() == END {
            skip = false;
        }
    }
    out
}

fn render_block(packs: &[&Pack], local_rules: &[LocalRule], version: &str) -> String {
    let mut s = String::new();
    s.push_str("<!-- handrail:begin (generated by Handrail; do not edit by hand, use the handrail command) -->\n");
    s.push_str(&format!("# Local policy (Handrail {version})\n\n"));
    s.push_str(
        "> This block is owned by root, loaded in every session, and cannot be excluded by user\n",
    );
    s.push_str(
        "> settings. Hard limits are enforced by managed-settings.d/ and hooks in the same\n",
    );
    s.push_str("> directory; this text is guidance for the agent. The two work together.\n");
    let mut sorted: Vec<&&Pack> = packs.iter().collect();
    sorted.sort_by_key(|p| p.id());
    for p in sorted {
        s.push('\n');
        s.push_str(p.rules.trim_end());
        s.push('\n');
    }
    if !local_rules.is_empty() {
        s.push_str("\n### Local rules\n\n");
        for r in local_rules {
            s.push_str(&format!("- {}\n", r.text));
        }
    }
    s.push_str(END);
    s.push('\n');
    s
}

/// Other policy sources that change whether our file-based policy is used at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// An MDM policy exists. Under Claude Code's default "first-wins" rule it takes
    /// precedence and file-based policy (ours) is ignored entirely.
    MdmPolicy(PathBuf),
    /// A managed-settings.json we did not write. It merges with our fragments.
    ForeignManagedSettings(PathBuf),
}

pub fn other_sources(target: &Target, user: Option<&str>) -> Vec<Finding> {
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        let mut candidates = vec![PathBuf::from(
            "/Library/Managed Preferences/com.anthropic.claudecode.plist",
        )];
        if let Some(u) = user {
            candidates.push(PathBuf::from(format!(
                "/Library/Managed Preferences/{u}/com.anthropic.claudecode.plist"
            )));
        }
        out.extend(
            candidates
                .into_iter()
                .filter(|p| p.exists())
                .map(Finding::MdmPolicy),
        );
    }
    let ms = target.managed_root.join("managed-settings.json");
    if ms.exists() {
        out.push(Finding::ForeignManagedSettings(ms));
    }
    out
}

#[cfg(test)]
mod tests;
