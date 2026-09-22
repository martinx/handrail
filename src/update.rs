//! `handrail self-update` and `handrail statusline`.

use crate::context::Ctx;
use std::path::Path;

const REPO: &str = "martinx/handrail";
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// The opt-in daily update check. Off unless you turn it on with `self-update --auto on`.
///
/// When on, `handrail statusline` shows `↑<version>` if a newer release exists. The status
/// line runs often, so it never waits on the network: it reads this cache, and when the cache
/// is older than a day it starts a background refresh and returns immediately. That refresh
/// is one GET to api.github.com per day — the only network traffic this adds.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct UpdateCheck {
    enabled: bool,
    checked_at: u64,
    latest: Option<String>,
}

fn cache_path(ctx: &Ctx) -> std::path::PathBuf {
    ctx.target.user_dir.join("handrail/update-check.json")
}

fn read_cache(ctx: &Ctx) -> UpdateCheck {
    std::fs::read(cache_path(ctx))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_cache(ctx: &Ctx, c: &UpdateCheck) {
    let p = cache_path(ctx);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&p, serde_json::to_vec_pretty(c).unwrap_or_default());
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Turn the daily check on or off.
pub fn set_auto(ctx: &Ctx, on: bool) -> Result<(), String> {
    let mut c = read_cache(ctx);
    c.enabled = on;
    if !on {
        c.latest = None;
        write_cache(ctx, &c);
        println!("Daily update check is off. Handrail makes no network requests unless you run self-update.");
        return Ok(());
    }
    // Check once now, so the status line is right from the start
    c.latest = latest_release()
        .ok()
        .map(|t| t.trim_start_matches('v').to_string());
    c.checked_at = now();
    write_cache(ctx, &c);
    println!("Daily update check is on: at most one request a day to api.github.com.");
    println!("When a newer release exists, `handrail statusline` shows ↑<version>. Nothing is installed automatically.");
    match &c.latest {
        Some(l) if newer_than_current(l) => {
            println!("A newer release is available now: {l}. Update with: handrail self-update")
        }
        Some(_) => println!("You are on the latest release."),
        None => println!("(Could not reach GitHub just now; it will try again tomorrow.)"),
    }
    Ok(())
}

/// Background refresh started by `statusline`. Silent; failures just wait for tomorrow.
pub fn refresh_cache(ctx: &Ctx) {
    let mut c = read_cache(ctx);
    if !c.enabled {
        return;
    }
    if let Ok(t) = latest_release() {
        c.latest = Some(t.trim_start_matches('v').to_string());
    }
    c.checked_at = now();
    write_cache(ctx, &c);
}

fn newer_than_current(latest: &str) -> bool {
    crate::core::version::at_least(crate::change::VERSION, latest) == Some(false)
}

/// `↑0.1.2` when the cache knows of a newer release; starts a background refresh when the
/// cache is stale. Never blocks.
fn update_hint(ctx: &Ctx) -> String {
    let mut c = read_cache(ctx);
    if !c.enabled {
        return String::new();
    }
    if now().saturating_sub(c.checked_at) > CHECK_INTERVAL_SECS {
        // Claim this round first, so a status line redrawn every second starts one refresh, not many
        c.checked_at = now();
        write_cache(ctx, &c);
        if let Ok(exe) = std::env::current_exe() {
            let mut cmd = std::process::Command::new(exe);
            cmd.arg("__refresh-update-check");
            if ctx.custom_paths {
                cmd.arg("--user-dir").arg(&ctx.target.user_dir);
            }
            let _ = cmd
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    }
    match c.latest {
        Some(l) if newer_than_current(&l) => format!(" ↑{l}"),
        _ => String::new(),
    }
}
const INSTALLER: &str = "https://handrail.bitey.ai/install.sh";

/// How this binary was installed decides who is allowed to update it.
enum Installed {
    Homebrew,
    Cargo,
    Script,
}

fn installed_by(exe: &Path) -> Installed {
    let s = exe.to_string_lossy();
    if s.contains("/Cellar/") || s.contains("/homebrew/") || s.contains("/linuxbrew/") {
        Installed::Homebrew
    } else if s.contains("/.cargo/bin/") {
        Installed::Cargo
    } else {
        Installed::Script
    }
}

/// The newest release tag on GitHub, e.g. "v0.1.1". Uses the system curl, so Handrail
/// needs no HTTP stack of its own. This is the only network request Handrail makes, and
/// it only happens when you run `self-update`.
fn latest_release() -> Result<String, String> {
    let out = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "20",
            "-H",
            "Accept: application/vnd.github+json",
        ])
        .arg(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .output()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "could not reach GitHub: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("unexpected reply from GitHub: {e}"))?;
    v["tag_name"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "GitHub reported no release".into())
}

pub fn self_update(ctx: &Ctx, check_only: bool) -> Result<(), String> {
    let current = crate::change::VERSION;
    let tag = latest_release()?;
    let latest = tag.trim_start_matches('v');
    let mut c = read_cache(ctx);
    if c.enabled {
        c.latest = Some(latest.to_string());
        c.checked_at = now();
        write_cache(ctx, &c);
    }
    let newer = crate::core::version::at_least(current, latest) == Some(false);
    if !newer {
        if current == latest {
            println!("handrail {current} is the latest release.");
        } else {
            println!(
                "You have handrail {current}; the latest release is {latest}. Nothing to update."
            );
        }
        return Ok(());
    }
    println!("handrail {latest} is available (you have {current}).");
    if check_only {
        println!("Update with: handrail self-update");
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| format!("locating the handrail binary: {e}"))?;
    match installed_by(&exe) {
        // Package managers own their files; updating behind their back breaks them
        Installed::Homebrew => {
            println!("Installed with Homebrew. Update with: brew upgrade handrail")
        }
        Installed::Cargo => println!("Installed with cargo. Update with: cargo install handrail"),
        Installed::Script => {
            let dir = exe
                .parent()
                .ok_or("the handrail binary has no parent directory")?;
            println!("Updating {} with the installer (checksum-verified; sudo only for the final copy).\n", exe.display());
            // Pin the version we just reported, so what gets installed is what the user saw
            let status = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("curl -fsSL {INSTALLER} | sh"))
                .env("HANDRAIL_VERSION", &tag)
                .env("HANDRAIL_INSTALL_DIR", dir)
                .status()
                .map_err(|e| format!("could not run the installer: {e}"))?;
            if !status.success() {
                return Err(
                    "the update did not complete; the previous version is unchanged".into(),
                );
            }
        }
    }
    Ok(())
}

/// One short line for Claude Code's status line. Local reads only; no network.
///
/// `handrail: baseline ✓`, `handrail: 4 packs ✓`, `handrail: drift ⚠`, `handrail: off`
pub fn statusline(ctx: &Ctx) -> String {
    let intent = ctx.current_intent();
    let hint = update_hint(ctx);
    if intent.packs.is_empty() && intent.local_rules.is_empty() {
        return format!("handrail: off{hint}");
    }
    for root in [&ctx.target.managed_root, &ctx.target.user_dir] {
        if root.join(crate::core::apply::JOURNAL).exists() {
            return format!("handrail: interrupted ⚠{hint}");
        }
    }
    let label = ctx
        .catalog
        .profiles
        .values()
        .find(|p| {
            p.packs
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                == intent.packs
        })
        .map(|p| p.name.clone())
        .unwrap_or_else(|| format!("{} packs", intent.packs.len()));
    // Same comparison as status and doctor
    let c = ctx.compare();
    if c.drift() || c.error.is_some() {
        format!("handrail: {label} drift ⚠{hint}")
    } else if c.outdated() {
        format!("handrail: {label} ✓ re-apply{hint}")
    } else {
        format!("handrail: {label} ✓{hint}")
    }
}
