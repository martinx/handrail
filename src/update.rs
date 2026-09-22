//! `handrail self-update` and `handrail statusline`.

use crate::context::Ctx;
use std::path::Path;

const REPO: &str = "martinx/handrail";
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

pub fn self_update(check_only: bool) -> Result<(), String> {
    let current = crate::change::VERSION;
    let tag = latest_release()?;
    let latest = tag.trim_start_matches('v');
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
    if intent.packs.is_empty() && intent.local_rules.is_empty() {
        return "handrail: off".into();
    }
    for root in [&ctx.target.managed_root, &ctx.target.user_dir] {
        if root.join(crate::core::apply::JOURNAL).exists() {
            return "handrail: interrupted ⚠".into();
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
    // Same drift check as doctor: re-plan what is installed; anything to do means the
    // files on disk are not what Handrail wrote
    let drift = crate::claude::plan(&ctx.catalog, &ctx.target, &intent, crate::change::VERSION)
        .map(|p| {
            [&p.enforced, &p.advisory].iter().any(|pl| {
                pl.ops
                    .iter()
                    .any(|o| !matches!(o, crate::core::plan::Op::RemoveDirIfEmpty { .. }))
            })
        })
        .unwrap_or(true);
    if drift {
        format!("handrail: {label} drift ⚠")
    } else {
        format!("handrail: {label} ✓")
    }
}
