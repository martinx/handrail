//! Where things are on this machine, and what is installed right now.

use handrail_claude::{read_state, Intent, State, Target};
use handrail_core::catalog::Catalog;
use std::path::{Path, PathBuf};

pub struct Ctx {
    pub catalog: Catalog,
    pub target: Target,
    pub claude_version: Option<String>,
    /// Set when --managed-root / --user-dir were given. Test and development use only.
    pub custom_paths: bool,
}

#[derive(Default)]
pub struct Overrides {
    pub managed_root: Option<PathBuf>,
    pub user_dir: Option<PathBuf>,
    pub claude_version: Option<String>,
}

impl Ctx {
    pub fn new(o: Overrides) -> Result<Ctx, String> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("HOME is not set")?;
        let mut target = Target::system(&home)?;
        let custom_paths = o.managed_root.is_some() || o.user_dir.is_some();
        if let Some(r) = o.managed_root {
            target.managed_root = r;
        }
        if let Some(u) = o.user_dir {
            target.user_dir = u;
        }
        let claude_version = o.claude_version.or_else(detect_claude_version);
        Ok(Ctx {
            catalog: crate::embedded::catalog(),
            target,
            claude_version,
            custom_paths,
        })
    }

    pub fn enforced_state(&self) -> Option<State> {
        read_state(&self.target.managed_root)
    }

    pub fn advisory_state(&self) -> Option<State> {
        read_state(&self.target.user_dir)
    }

    /// What is installed now, as an intent. Packs of both tiers, plus local rules.
    pub fn current_intent(&self) -> Intent {
        let mut i = Intent {
            claude_version: self.claude_version.clone(),
            ..Default::default()
        };
        if let Some(s) = self.enforced_state() {
            i.packs.extend(s.packs.into_iter().map(|p| p.id));
            i.local_rules = s.local_rules;
        }
        if let Some(s) = self.advisory_state() {
            i.packs.extend(s.packs.into_iter().map(|p| p.id));
        }
        i
    }

    pub fn is_installed(&self, id: &str) -> bool {
        self.current_intent().packs.contains(id)
    }
}

/// `claude --version`, or `None` when Claude Code is not on PATH.
fn detect_claude_version() -> Option<String> {
    let out = std::process::Command::new("claude")
        .arg("--version")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// True when the current user can write `path` (or, if it does not exist yet, create it).
pub fn can_write(path: &Path) -> bool {
    let mut p = Some(path);
    while let Some(dir) = p {
        if dir.exists() {
            let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()) else {
                return false;
            };
            // SAFETY: `c` is a valid NUL-terminated string for the duration of the call.
            return unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0;
        }
        p = dir.parent();
    }
    false
}

pub fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}
