//! Read-only commands: list, show, profiles, status, doctor.

use crate::claude::{other_sources, Finding, TARGET};
use crate::context::{can_write, Ctx};
use crate::core::apply::JOURNAL;
use crate::core::catalog::{Enforcement, Origin, Pack, Tier};
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
            let from = match &p.origin {
                Origin::Builtin => String::new(),
                Origin::External(o) => format!("  [from {o}]"),
            };
            println!(
                "  {mark} {:<14} {:<9} {:<10} {}{from}",
                p.id(),
                tier(p.manifest.tier),
                enforcement(p).as_str(),
                p.manifest.title
            );
        }
        println!();
    }
    if category.is_none() && !ctx.catalog.profiles.is_empty() {
        // Profiles are a separate concept (named sets of these packs): one line here,
        // the details in `handrail profiles`
        let names: Vec<String> = by_size(ctx)
            .into_iter()
            .map(|p| {
                if matches_installed(&p.packs, &installed) {
                    format!("{} (in use)", p.name)
                } else {
                    p.name.clone()
                }
            })
            .collect();
        println!(
            "Profiles: {}  — sets of these packs; see: handrail profiles\n",
            names.join(", ")
        );
    }
    println!("Details: handrail show <pack>");
}

/// Profiles from the smallest to the largest, which reads as mildest to strictest.
fn by_size(ctx: &Ctx) -> Vec<&crate::core::catalog::Profile> {
    let mut v: Vec<_> = ctx.catalog.profiles.values().collect();
    v.sort_by_key(|p| p.packs.len());
    v
}

/// True when the installed packs are exactly this profile's packs.
fn matches_installed(packs: &[String], installed: &std::collections::BTreeSet<String>) -> bool {
    !installed.is_empty()
        && packs.len() == installed.len()
        && packs.iter().all(|p| installed.contains(p))
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
    if let Origin::External(o) = &p.origin {
        println!("From: {o} (not part of handrail's built-in catalog; review it before enabling)");
    }
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
    let installed = ctx.current_intent().packs;
    println!("Profiles are named sets of packs. `handrail use <profile>` makes the installed packs exactly that set.\n");
    for p in by_size(ctx) {
        let mark = if matches_installed(&p.packs, &installed) {
            "*"
        } else {
            " "
        };
        println!("{mark} {}  — {}", p.name, p.description);
        println!("    {}", p.packs.join(" "));
        if !installed.is_empty() && mark == " " {
            let add: Vec<&str> = p
                .packs
                .iter()
                .filter(|x| !installed.contains(*x))
                .map(|x| x.as_str())
                .collect();
            let remove: Vec<&str> = installed
                .iter()
                .filter(|x| !p.packs.contains(x))
                .map(|x| x.as_str())
                .collect();
            if !add.is_empty() {
                println!("    switching adds: {}", add.join(" "));
            }
            if !remove.is_empty() {
                println!("    switching removes: {}", remove.join(" "));
            }
        }
        println!();
    }
    println!("* = exactly what is installed.  Apply one: handrail use <profile> --dry-run");
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
    let c = ctx.compare();
    let files = |c: &crate::context::Comparison| {
        c.changed
            .iter()
            .map(|(t, p)| format!("{t}: {p}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if c.outdated() {
        let list: Vec<String> = c
            .updated
            .iter()
            .map(|(id, old, new)| format!("{id} {old} → {new}"))
            .collect();
        out.push(format!(
            "Pack updates available: {}. Apply them with: handrail use <profile> (or handrail enable <pack>)",
            list.join(", ")
        ));
    } else if c.drift() {
        out.push(format!(
            "Installed files differ from what Handrail wrote ({}): edited or deleted by something else. Re-apply to restore: handrail enable <pack> (or handrail doctor for details)",
            files(&c)
        ));
    }
    if let Some(e) = c.error {
        out.push(format!("Cannot compare with what is installed: {e}"));
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

/// The catalog as JSON: the single source for anything that presents packs (the website).
pub fn catalog_json(ctx: &Ctx) -> String {
    let packs: Vec<serde_json::Value> = ctx
        .catalog
        .packs
        .values()
        .map(|p| {
            let m = &p.manifest;
            let targets: serde_json::Map<String, serde_json::Value> = p
                .targets
                .iter()
                .map(|(t, f)| {
                    (t.clone(), serde_json::json!({ "enforcement": f.spec.enforcement.as_str(), "min_version": f.spec.min_version }))
                })
                .collect();
            serde_json::json!({
                "id": m.id, "version": m.version, "category": m.category, "tier": tier(m.tier),
                "title": m.title, "summary": m.summary, "protects": m.protects,
                "tradeoffs": m.tradeoffs, "limits": m.limits, "targets": targets,
            })
        })
        .collect();
    let profiles: Vec<serde_json::Value> = ctx
        .catalog
        .profiles
        .values()
        .map(|p| serde_json::json!({ "name": p.name, "description": p.description, "packs": p.packs }))
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({ "version": crate::change::VERSION, "packs": packs, "profiles": profiles }))
        .expect("catalog serialises")
}
