//! Read-only commands: list, show, profiles, status, doctor.

use crate::context::{can_write, Ctx};
use handrail_claude::{other_sources, plan, Finding, TARGET};
use handrail_core::apply::JOURNAL;
use handrail_core::catalog::{Enforcement, Pack, Tier};
use std::collections::BTreeMap;

fn enforcement(p: &Pack) -> Enforcement {
    p.targets
        .get(TARGET)
        .map(|t| t.spec.enforcement)
        .unwrap_or(Enforcement::Unsupported)
}

fn tier(t: Tier) -> &'static str {
    match t {
        Tier::Enforced => "enforced",
        Tier::Advisory => "advisory",
    }
}

pub fn list(ctx: &Ctx, category: Option<&str>) {
    let installed = ctx.current_intent().packs;
    let mut by_cat: BTreeMap<&str, Vec<&Pack>> = BTreeMap::new();
    for p in ctx.catalog.packs.values() {
        if category.is_none_or(|c| c == p.manifest.category) {
            by_cat
                .entry(p.manifest.category.as_str())
                .or_default()
                .push(p);
        }
    }
    if by_cat.is_empty() {
        println!("No packs in that category.");
        return;
    }
    println!(
        "Packs for Claude Code (tier · how strongly Claude Code enforces it). * = installed\n"
    );
    for (cat, packs) in by_cat {
        println!("{cat}");
        for p in packs {
            let mark = if installed.contains(p.id()) { "*" } else { " " };
            println!(
                "  {mark} {:<14} {:<9} {:<10} {}",
                p.id(),
                tier(p.manifest.tier),
                enforcement(p).as_str(),
                p.manifest.title
            );
        }
        println!();
    }
    println!("Details: handrail show <pack>    Profiles: handrail profiles");
}

pub fn show(ctx: &Ctx, id: &str) -> Result<(), String> {
    let p = ctx
        .catalog
        .packs
        .get(id)
        .ok_or_else(|| format!("no pack named \"{id}\" (see: handrail list)"))?;
    let m = &p.manifest;
    println!("{}  {} (v{})", m.id, m.title, m.version);
    println!(
        "Category: {}   Tier: {}   Installed: {}",
        m.category,
        tier(m.tier),
        if ctx.is_installed(id) { "yes" } else { "no" }
    );
    println!("\n{}", m.summary);
    if !m.protects.is_empty() {
        println!("\nProtects against:");
        m.protects.iter().for_each(|x| println!("  - {x}"));
    }
    if !m.tradeoffs.is_empty() {
        println!("\nTradeoffs:");
        m.tradeoffs.iter().for_each(|x| println!("  - {x}"));
    }
    println!("\nLimits: {}", m.limits);
    println!("\nPer agent:");
    for (t, files) in &p.targets {
        let min = files
            .spec
            .min_version
            .as_deref()
            .map(|v| format!(", needs {v}+"))
            .unwrap_or_default();
        println!("  {t}: {}{min}", files.spec.enforcement.as_str());
    }
    let in_profiles: Vec<&str> = ctx
        .catalog
        .profiles
        .values()
        .filter(|pr| pr.packs.iter().any(|x| x == id))
        .map(|pr| pr.name.as_str())
        .collect();
    if !in_profiles.is_empty() {
        println!("\nIn profiles: {}", in_profiles.join(", "));
    }
    Ok(())
}

pub fn profiles(ctx: &Ctx) {
    for p in ctx.catalog.profiles.values() {
        println!("{}  — {}", p.name, p.description);
        println!("    {}\n", p.packs.join(" "));
    }
    println!("Apply one: handrail use <profile> --dry-run");
}

pub fn status(ctx: &Ctx) {
    println!(
        "Handrail {} · Claude Code {}",
        crate::change::VERSION,
        ctx.claude_version.as_deref().unwrap_or("not found on PATH")
    );
    let e = ctx.enforced_state();
    let a = ctx.advisory_state();
    match &e {
        Some(s) if !s.packs.is_empty() || !s.local_rules.is_empty() => {
            println!("\nEnforced tier ({}):", ctx.target.managed_root.display());
            for p in &s.packs {
                let enf = ctx
                    .catalog
                    .packs
                    .get(&p.id)
                    .map(|x| enforcement(x).as_str())
                    .unwrap_or("unknown pack");
                println!("  + {:<14} v{:<7} {enf}", p.id, p.version);
            }
            if !s.local_rules.is_empty() {
                println!("  local rules: {}", s.local_rules.len());
            }
        }
        _ => println!("\nEnforced tier: nothing installed"),
    }
    match &a {
        Some(s) if !s.packs.is_empty() => {
            println!("\nAdvisory tier ({}):", ctx.target.user_dir.display());
            s.packs
                .iter()
                .for_each(|p| println!("  + {:<14} v{}", p.id, p.version));
        }
        _ => println!("Advisory tier: nothing installed"),
    }
    let warnings = findings(ctx);
    if !warnings.is_empty() {
        println!();
        warnings.iter().for_each(|w| println!("! {w}"));
    }
    println!("\nTo confirm Claude Code picked it up: run /status in a new session (see \"Setting sources\"), or: claude doctor");
}

/// Problems worth telling the user about, shared by status and doctor.
fn findings(ctx: &Ctx) -> Vec<String> {
    let mut out = Vec::new();
    let user = std::env::var("USER").ok();
    for f in other_sources(&ctx.target, user.as_deref()) {
        match f {
            Finding::MdmPolicy(p) => out.push(format!(
                "An MDM-delivered Claude Code policy exists ({}). By default it takes precedence and Handrail's file-based policy is IGNORED. Ask your administrator to set managedSourcesBehavior to \"merge\".",
                p.display()
            )),
            Finding::ForeignManagedSettings(p) => {
                out.push(format!("{} exists (not written by Handrail); it is merged with Handrail's fragments.", p.display()))
            }
        }
    }
    for root in [&ctx.target.managed_root, &ctx.target.user_dir] {
        if root.join(JOURNAL).exists() {
            out.push(format!(
                "An interrupted run was detected in {}; the next change rolls it back first.",
                root.display()
            ));
        }
    }
    // Installed by another version: re-planning would differ for that reason alone
    let installed_by = ctx
        .enforced_state()
        .map(|s| s.handrail_version)
        .filter(|v| v != crate::change::VERSION);
    if let Some(v) = installed_by {
        out.push(format!("Installed with Handrail {v}; this is {}. Re-apply to update: handrail use <profile> or handrail enable <pack>", crate::change::VERSION));
        return out;
    }
    // Drift: re-plan what is installed. Anything to do means the files on disk are not
    // what Handrail wrote — edited, deleted, or tampered with.
    let intent = ctx.current_intent();
    if let Ok(p) = plan(&ctx.catalog, &ctx.target, &intent, crate::change::VERSION) {
        for (name, pl) in [("enforced", &p.enforced), ("advisory", &p.advisory)] {
            let changed: Vec<&str> = pl
                .ops
                .iter()
                .filter(|o| !matches!(o, handrail_core::plan::Op::RemoveDirIfEmpty { .. }))
                .map(|o| o.path())
                .collect();
            if !changed.is_empty() && (!intent.packs.is_empty() || !intent.local_rules.is_empty()) {
                out.push(format!(
                    "The {name} tier differs from what Handrail wrote ({}). Re-apply with: handrail enable <pack> (or handrail doctor for details)",
                    changed.join(", ")
                ));
            }
        }
    }
    out
}

pub fn doctor(ctx: &Ctx) {
    let mut ok = true;
    let mut check = |label: &str, good: bool, detail: String| {
        println!(
            "{} {label}{}",
            if good { "ok  " } else { "FAIL" },
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        );
        ok &= good;
    };
    check(
        "Claude Code on PATH",
        ctx.claude_version.is_some(),
        ctx.claude_version
            .clone()
            .unwrap_or_else(|| "not found".into()),
    );
    let root = &ctx.target.managed_root;
    check("managed policy directory", true, root.display().to_string());
    if root.exists() && !ctx.custom_paths {
        check(
            "managed directory is not writable by you",
            !can_write(root),
            String::new(),
        );
    }
    if let Ok(exe) = std::env::current_exe() {
        let writable = can_write(&exe) || exe.parent().is_some_and(can_write);
        check(
            "handrail binary is not writable by you",
            !writable,
            if writable {
                format!("{} — anything running as you could replace it before sudo runs it; install it where only root can write", exe.display())
            } else {
                exe.display().to_string()
            },
        );
    }
    let f = findings(ctx);
    check(
        "no conflicting policy sources, interruptions or drift",
        f.is_empty(),
        String::new(),
    );
    f.iter().for_each(|w| println!("     - {w}"));
    println!(
        "\n{}",
        if ok {
            "Everything looks right."
        } else {
            "Some checks failed; see above."
        }
    );
}
