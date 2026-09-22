//! Changing what is installed: plan, show, confirm, apply — elevating only for the
//! enforced tier.

use crate::claude::{plan, Intent, Plans};
use crate::context::{can_write, is_root, Ctx};
use crate::core::apply::{apply, recover};
use crate::core::catalog::Enforcement;
use crate::core::plan::{Op, Plan};
use std::collections::BTreeSet;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Opts {
    pub dry_run: bool,
    pub yes: bool,
}

pub fn run(ctx: &Ctx, new: Intent, opts: &Opts) -> Result<(), String> {
    let old = ctx.current_intent();
    let plans = plan(&ctx.catalog, &ctx.target, &new, VERSION).map_err(|e| e.to_string())?;
    describe(ctx, &old, &new, &plans);

    if plans.enforced.is_empty() && plans.advisory.is_empty() {
        println!("Nothing to change.");
        return Ok(());
    }
    if opts.dry_run {
        print_ops("Enforced tier", &ctx.target.managed_root, &plans.enforced);
        print_ops("Advisory tier", &ctx.target.user_dir, &plans.advisory);
        println!("\n[dry run] No files were written.");
        return Ok(());
    }
    confirm(opts.yes)?;

    if !plans.enforced.is_empty() {
        apply_enforced(ctx, &new, &plans.enforced)?;
    }
    if !plans.advisory.is_empty() {
        let root = &ctx.target.user_dir;
        if let Some(r) = recover(root).map_err(|e| e.to_string())? {
            println!(
                "Recovered an interrupted run in {} ({} restored, {} removed).",
                root.display(),
                r.restored,
                r.removed
            );
        }
        std::fs::create_dir_all(root).map_err(|e| format!("creating {}: {e}", root.display()))?;
        let backup = backup_dir(root, plans.advisory_packs.is_empty());
        apply(root, &plans.advisory, &backup).map_err(|e| e.to_string())?;
        prune_backups(root);
        if plans.advisory_packs.is_empty() {
            // Only our own subdirectories; ~/.claude itself belongs to Claude Code
            let _ = std::fs::remove_dir_all(root.join("handrail/backups"));
            let _ = std::fs::remove_dir(root.join("handrail"));
            let _ = std::fs::remove_dir(root.join("rules"));
        }
    }
    println!("\nDone. Takes effect in new claude sessions; running sessions are not affected.");
    println!("To confirm Claude Code picked it up: run /status in a new session (see \"Setting sources\"), or: claude doctor");
    Ok(())
}

fn describe(ctx: &Ctx, old: &Intent, new: &Intent, plans: &Plans) {
    let added: Vec<&String> = new.packs.difference(&old.packs).collect();
    let removed: Vec<&String> = old.packs.difference(&new.packs).collect();
    for id in &added {
        let Some(p) = ctx.catalog.packs.get(*id) else {
            continue;
        };
        let enf = p
            .targets
            .get(crate::claude::TARGET)
            .map(|t| t.spec.enforcement)
            .unwrap_or(Enforcement::Unsupported);
        println!(
            "\n+ {id}  {}  [{} · {}]",
            p.manifest.title,
            tier_name(p.manifest.tier),
            enf.as_str()
        );
        println!("    {}", p.manifest.summary);
        for t in &p.manifest.tradeoffs {
            println!("    Tradeoff: {t}");
        }
    }
    for id in &removed {
        let title = ctx
            .catalog
            .packs
            .get(*id)
            .map(|p| p.manifest.title.as_str())
            .unwrap_or("");
        println!("\n- {id}  {title}");
    }
    let (ra, rr) = (
        new.local_rules
            .iter()
            .filter(|r| !old.local_rules.contains(r))
            .count(),
        old.local_rules
            .iter()
            .filter(|r| !new.local_rules.contains(r))
            .count(),
    );
    if ra + rr > 0 {
        println!("\nLocal rules: {ra} added, {rr} removed");
    }
    for s in &plans.skipped {
        println!("\n! skipped {}: {}", s.id, s.reason);
    }
    if new.claude_version.is_none() && !new.packs.is_empty() {
        println!(
            "\n! Claude Code was not found on PATH; version requirements could not be checked."
        );
    }
    if !plans.enforced.is_empty() {
        println!(
            "\nEnforced tier: {} change(s) in {} (administrator rights required)",
            plans.enforced.ops.len(),
            ctx.target.managed_root.display()
        );
    }
    if !plans.advisory.is_empty() {
        println!(
            "Advisory tier: {} change(s) in {}",
            plans.advisory.ops.len(),
            ctx.target.user_dir.display()
        );
    }
}

fn tier_name(t: crate::core::catalog::Tier) -> &'static str {
    match t {
        crate::core::catalog::Tier::Enforced => "enforced tier",
        crate::core::catalog::Tier::Advisory => "advisory tier",
    }
}

fn print_ops(title: &str, root: &Path, plan: &Plan) {
    if plan.is_empty() {
        return;
    }
    println!("\n{title} ({}):", root.display());
    for op in &plan.ops {
        match op {
            Op::Write { path, .. } => println!("  write   {path}"),
            Op::Delete { path, .. } => println!("  delete  {path}"),
            Op::RemoveDirIfEmpty { path } => println!("  rmdir   {path} (if empty)"),
        }
    }
}

fn confirm(yes: bool) -> Result<(), String> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(
            "No terminal to confirm on. Review the plan with --dry-run, then re-run with --yes."
                .into(),
        );
    }
    print!("\nContinue? [y/N] ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).ok();
    match line.trim() {
        "y" | "Y" | "yes" | "YES" => Ok(()),
        _ => Err("Cancelled. Nothing was changed.".into()),
    }
}

/// Applies the enforced plan: directly when this process may write the managed directory
/// (running as root, or a custom test directory), otherwise through `sudo`.
fn apply_enforced(ctx: &Ctx, intent: &Intent, plan: &Plan) -> Result<(), String> {
    let root = &ctx.target.managed_root;
    if is_root() || (ctx.custom_paths && can_write(root)) {
        return apply_locally(root, plan, intent);
    }
    elevate(ctx, intent, &plan.hash())
}

pub fn apply_locally(root: &Path, plan: &Plan, intent: &Intent) -> Result<(), String> {
    if let Some(r) = recover(root).map_err(|e| e.to_string())? {
        println!(
            "Recovered an interrupted run in {} ({} restored, {} removed).",
            root.display(),
            r.restored,
            r.removed
        );
        return Err("An interrupted run was rolled back first. Re-run the command to review the plan against the restored files.".into());
    }
    std::fs::create_dir_all(root).map_err(|e| format!("creating {}: {e}", root.display()))?;
    let removing_all = intent.packs.is_empty() && intent.local_rules.is_empty();
    let backup = backup_dir(root, removing_all);
    let out = apply(root, plan, &backup).map_err(|e| e.to_string())?;
    prune_backups(root);
    if removing_all {
        remove_our_leftovers(root);
    }
    println!(
        "Applied to {}: {} written, {} removed.",
        root.display(),
        out.written,
        out.deleted
    );
    if removing_all {
        println!(
            "Backup of the removed files: {} (temporary directory)",
            backup.display()
        );
    }
    Ok(())
}

/// Runs `sudo <this binary> __apply` with the intent — not the file contents.
///
/// The privileged process recomputes the plan from the catalog compiled into the binary
/// and refuses unless its hash matches the plan the user just reviewed.
fn elevate(ctx: &Ctx, intent: &Intent, expect: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("locating the handrail binary: {e}"))?;
    if can_write(&exe) || exe.parent().is_some_and(can_write) {
        eprintln!(
            "\n! {} is writable by your user. Anything running as you could replace it before sudo runs it.\n  For enforced policy, install handrail somewhere only root can write (see: handrail doctor).",
            exe.display()
        );
    }
    let path = std::env::temp_dir().join(format!("handrail-intent-{}.json", std::process::id()));
    write_private(
        &path,
        &serde_json::to_vec_pretty(intent).expect("intent serialises"),
    )?;
    let mut cmd = std::process::Command::new("sudo");
    cmd.arg(&exe)
        .arg("__apply")
        .arg("--intent")
        .arg(&path)
        .arg("--expect")
        .arg(expect);
    if ctx.custom_paths {
        cmd.arg("--managed-root").arg(&ctx.target.managed_root);
    }
    println!("\nAdministrator rights are needed for the enforced tier; running sudo.");
    let status = cmd.status();
    let _ = std::fs::remove_file(&path);
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => Err(
            "The enforced tier was not changed (sudo or the privileged step failed; see above)."
                .into(),
        ),
        Err(e) => Err(format!("could not run sudo: {e}")),
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("creating {}: {e}", path.display()))?;
    f.write_all(bytes)
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

/// The privileged half: `sudo handrail __apply --intent <file> --expect <hash>`.
pub fn privileged_apply(ctx: &Ctx, intent_path: &Path, expect: &str) -> Result<(), String> {
    let bytes = std::fs::read(intent_path)
        .map_err(|e| format!("reading {}: {e}", intent_path.display()))?;
    let intent: Intent =
        serde_json::from_slice(&bytes).map_err(|e| format!("parsing the intent: {e}"))?;
    let root = &ctx.target.managed_root;
    if let Some(r) = recover(root).map_err(|e| e.to_string())? {
        println!(
            "Recovered an interrupted run ({} restored, {} removed).",
            r.restored, r.removed
        );
    }
    let plans = plan(&ctx.catalog, &ctx.target, &intent, VERSION).map_err(|e| e.to_string())?;
    if plans.enforced.hash() != expect {
        return Err("The plan changed between review and apply (a file was modified, or the request was altered). Nothing was changed; run the command again.".into());
    }
    apply_locally(root, &plans.enforced, &intent)
}

/// Undo the last change: restore the intent recorded in the newest backup.
pub fn rollback(ctx: &Ctx, opts: &Opts) -> Result<(), String> {
    let dir = ctx.target.managed_root.join("handrail/backups");
    let newest = std::fs::read_dir(&dir)
        .ok()
        .and_then(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).max())
        .ok_or("Nothing to roll back to: there are no backups (removing every pack also removes its backups).")?;
    let prev: crate::claude::State = std::fs::read(newest.join("handrail/state.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let current = ctx.current_intent();
    let advisory: BTreeSet<String> = current
        .packs
        .iter()
        .filter(|id| {
            ctx.catalog
                .packs
                .get(*id)
                .is_some_and(|p| p.manifest.tier == crate::core::catalog::Tier::Advisory)
        })
        .cloned()
        .collect();
    let mut intent = Intent {
        claude_version: ctx.claude_version.clone(),
        local_rules: prev.local_rules,
        ..Default::default()
    };
    intent.packs.extend(prev.packs.into_iter().map(|p| p.id));
    intent.packs.extend(advisory);
    println!(
        "Rolling back to the state before the last change ({}).",
        newest.file_name().unwrap_or_default().to_string_lossy()
    );
    run(ctx, intent, opts)
}

/// After removing everything: earlier backups kept for rollback would otherwise keep
/// `handrail/` alive. "Remove everything" means nothing of ours stays in the directory.
/// The files removed by this run are backed up in the temporary directory instead.
fn remove_our_leftovers(root: &Path) {
    let _ = std::fs::remove_dir_all(root.join("handrail/backups"));
    let _ = std::fs::remove_dir(root.join("handrail"));
    let _ = std::fs::remove_dir(root.join("managed-settings.d"));
    // The directory itself only if nobody else has anything in it
    let _ = std::fs::remove_dir(root);
}

fn backup_dir(root: &Path, outside: bool) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if outside {
        // Removing everything must not leave backups behind in the managed directory
        std::env::temp_dir().join(format!("handrail-backup-{ts}-{}", std::process::id()))
    } else {
        root.join("handrail/backups")
            .join(format!("{ts}-{}", std::process::id()))
    }
}

/// Keeps the five newest backups.
fn prune_backups(root: &Path) {
    let dir = root.join("handrail/backups");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut all: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    all.sort();
    let excess = all.len().saturating_sub(5);
    for old in all.into_iter().take(excess) {
        let _ = std::fs::remove_dir_all(old);
    }
}
