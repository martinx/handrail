//! Where things are on this machine, and what is installed right now.

use crate::claude::{read_state, Intent, State, Target, EXTERNAL};
use crate::core::catalog::{Catalog, DirSource, Origin, Tier};
use std::path::{Path, PathBuf};

pub struct Ctx {
    pub catalog: Catalog,
    pub target: Target,
    pub claude_version: Option<String>,
    /// Set when --managed-root / --user-dir were given. Test and development use only.
    pub custom_paths: bool,
    /// The --catalog sources, resolved to local directories, in the order given.
    pub catalogs: Vec<ExternalCatalog>,
}

/// One --catalog source: what the user typed, and the local directory holding it.
pub struct ExternalCatalog {
    pub label: String,
    pub dir: PathBuf,
    /// A temporary clone, removed when handrail exits.
    clone: bool,
}

impl Drop for ExternalCatalog {
    fn drop(&mut self) {
        if self.clone {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

#[derive(Default)]
pub struct Overrides {
    pub managed_root: Option<PathBuf>,
    pub user_dir: Option<PathBuf>,
    pub claude_version: Option<String>,
    /// `--catalog` values: a directory, or a git repository (`github.com/you/packs`,
    /// `https://…`, `git@…`, `file://…`).
    pub catalogs: Vec<String>,
    /// Labels for already-resolved `--catalog` directories, passed by the unprivileged
    /// process to `__apply` so the recorded origin is the repository, not a temp path.
    pub catalog_labels: Vec<String>,
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
        let mut catalog = crate::embedded::catalog();
        // Installed copies of external packs. The user-scope copy is writable by anything
        // running as the user, so it may only contribute advisory packs, it is loaded
        // before the root-owned copy (which therefore wins), and the privileged step
        // ignores it entirely.
        if !is_root() {
            load_installed(&mut catalog, &target.user_dir, Some(Tier::Advisory));
        }
        load_installed(&mut catalog, &target.managed_root, None);
        let mut catalogs = Vec::new();
        for (i, arg) in o.catalogs.iter().enumerate() {
            let c = resolve(arg, o.catalog_labels.get(i))?;
            let mut ext = Catalog::load(&DirSource(c.dir.clone())).map_err(|ps| {
                let list: Vec<String> = ps.iter().map(|p| format!("  {p}")).collect();
                format!("catalog {} has problems:\n{}", c.label, list.join("\n"))
            })?;
            ext.set_origin(Origin::External(c.label.clone()));
            catalog.merge(ext).map_err(|ps| {
                let list: Vec<String> = ps.iter().map(|p| format!("  {p}")).collect();
                format!("catalog {} cannot be used:\n{}", c.label, list.join("\n"))
            })?;
            catalogs.push(c);
        }
        Ok(Ctx {
            catalog,
            target,
            claude_version,
            custom_paths,
            catalogs,
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

/// Adds the installed copies of external packs under `root`, labelled with the origin
/// recorded in that tier's state. Problems are reported and the copy skipped: a broken
/// copy must not stop `status`, `disable` or `rollback` from working.
fn load_installed(catalog: &mut Catalog, root: &Path, only: Option<Tier>) {
    let dir = root.join(EXTERNAL);
    if !dir.join("packs").is_dir() {
        return;
    }
    let mut ext = match Catalog::load(&DirSource(dir.clone())) {
        Ok(c) => c,
        Err(ps) => {
            eprintln!("! ignoring installed external packs in {}:", dir.display());
            for p in ps {
                eprintln!("    {p}");
            }
            return;
        }
    };
    let origins: std::collections::BTreeMap<String, String> = read_state(root)
        .map(|s| {
            s.packs
                .into_iter()
                .filter_map(|p| Some((p.id, p.origin?)))
                .collect()
        })
        .unwrap_or_default();
    ext.packs
        .retain(|_, p| only.is_none_or(|t| p.manifest.tier == t));
    for (id, p) in ext.packs.iter_mut() {
        let label = origins
            .get(id)
            .cloned()
            .unwrap_or_else(|| "installed copy".into());
        p.origin = Origin::External(label);
    }
    if let Err(ps) = catalog.merge(ext) {
        eprintln!("! ignoring installed external packs in {}:", dir.display());
        for p in ps {
            eprintln!("    {p}");
        }
    }
}

/// Turns a --catalog value into a local directory, cloning it if it is a repository.
fn resolve(arg: &str, label: Option<&String>) -> Result<ExternalCatalog, String> {
    let path = Path::new(arg);
    if path.is_dir() {
        let dir = path
            .canonicalize()
            .map_err(|e| format!("reading {arg}: {e}"))?;
        let label = label.cloned().unwrap_or_else(|| dir.display().to_string());
        return Ok(ExternalCatalog {
            label,
            dir,
            clone: false,
        });
    }
    let url = if arg.contains("://") || arg.starts_with("git@") {
        arg.to_string()
    } else if arg.starts_with("github.com/") {
        format!("https://{arg}")
    } else {
        return Err(format!(
            "--catalog {arg}: not a directory, and not a repository URL (e.g. github.com/you/your-packs)"
        ));
    };
    let dir = std::env::temp_dir().join(format!(
        "handrail-catalog-{}-{}",
        std::process::id(),
        url.bytes()
            .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32))
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let out = std::process::Command::new("git")
        .args(["clone", "--quiet", "--depth", "1", "--", &url])
        .arg(&dir)
        .output()
        .map_err(|e| format!("--catalog {arg}: could not run git: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!(
            "--catalog {arg}: git clone failed:\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(ExternalCatalog {
        label: url,
        dir,
        clone: true,
    })
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
