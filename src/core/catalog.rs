//! Packs and profiles: loading and validation.
//!
//! A catalog is a directory tree:
//!
//! ```text
//! packs/<id>/pack.toml            metadata, tier, per-target specs
//! packs/<id>/rules.md             agent-facing instructions (English source of truth)
//! packs/<id>/<target>/settings.json
//! packs/<id>/<target>/hooks/*.sh
//! profiles/<name>.toml            named sets of packs
//! ```
//!
//! Validation reports every problem at once, each with the file and the reason, so a
//! contributor fixes a pack in one pass instead of one error per run.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

/// Where catalog files come from: a directory on disk, or files embedded in the binary.
pub trait Source {
    /// Reads a file by its path relative to the catalog root, `/`-separated.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    /// Names of the entries (files or directories) directly under `dir`, sorted.
    fn entries(&self, dir: &str) -> Vec<String>;
}

/// A catalog directory on disk. The binary uses the embedded catalog; tests load the
/// repository's `catalog/` (and fixtures) through this.
#[cfg(test)]
pub struct DirSource(pub PathBuf);

#[cfg(test)]
impl Source for DirSource {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(path)).ok()
    }
    fn entries(&self, dir: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.0.join(dir))
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| !n.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

/// Where a pack is installed and who can change it. See docs/design.md §3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Owned by root; changing it requires administrator authentication.
    Enforced,
    /// User scope; the user (and therefore the agent) can change it.
    Advisory,
}

/// How strongly a pack is enforced on one particular agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Enforcement {
    Enforced,
    Partial,
    Advisory,
    Unsupported,
}

impl Enforcement {
    pub fn as_str(self) -> &'static str {
        match self {
            Enforcement::Enforced => "enforced",
            Enforcement::Partial => "partial",
            Enforcement::Advisory => "advisory",
            Enforcement::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub category: String,
    pub tier: Tier,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub protects: Vec<String>,
    #[serde(default)]
    pub tradeoffs: Vec<String>,
    pub limits: String,
    #[serde(default)]
    pub targets: BTreeMap<String, TargetSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetSpec {
    pub enforcement: Enforcement,
    pub min_version: Option<String>,
    pub settings: Option<String>,
    #[serde(default)]
    pub hooks: Vec<String>,
}

/// A hook script shipped with a pack.
#[derive(Debug, Clone)]
pub struct Hook {
    /// File name, e.g. `guard.sh`. Installed under the pack's own directory.
    pub name: String,
    pub content: Vec<u8>,
}

/// Everything a pack provides for one agent.
#[derive(Debug, Clone)]
pub struct TargetFiles {
    pub spec: TargetSpec,
    pub settings: Option<serde_json::Value>,
    pub hooks: Vec<Hook>,
}

#[derive(Debug, Clone)]
pub struct Pack {
    pub manifest: Manifest,
    pub rules: String,
    pub targets: BTreeMap<String, TargetFiles>,
}

impl Pack {
    pub fn id(&self) -> &str {
        &self.manifest.id
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub description: String,
    pub packs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub packs: BTreeMap<String, Pack>,
    pub profiles: BTreeMap<String, Profile>,
}

/// One validation problem: which file, and what is wrong with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub file: String,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file, self.message)
    }
}

/// Placeholders a settings file may use. Anything else that looks like one is a typo.
pub const PLACEHOLDERS: &[&str] = &["@PACK_DIR@"];

fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn safe_relative(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

impl Catalog {
    /// Loads and validates a catalog. Returns every problem found, not just the first.
    pub fn load(src: &dyn Source) -> Result<Catalog, Vec<Problem>> {
        let mut problems = Vec::new();
        let mut packs = BTreeMap::new();
        for dir in src.entries("packs") {
            match load_pack(src, &dir, &mut problems) {
                Some(p) => {
                    packs.insert(p.manifest.id.clone(), p);
                }
                None => continue,
            }
        }
        let mut profiles = BTreeMap::new();
        for file in src.entries("profiles") {
            let Some(name) = file.strip_suffix(".toml") else {
                continue;
            };
            let path = format!("profiles/{file}");
            let Some(bytes) = src.read(&path) else {
                continue;
            };
            match toml::from_str::<Profile>(&String::from_utf8_lossy(&bytes)) {
                Ok(p) => {
                    if p.name != name {
                        problems.push(prob(
                            &path,
                            format!("name \"{}\" must match the file name \"{name}\"", p.name),
                        ));
                    }
                    let mut seen = std::collections::BTreeSet::new();
                    for id in &p.packs {
                        if !packs.contains_key(id) {
                            problems.push(prob(
                                &path,
                                format!("refers to pack \"{id}\", which does not exist"),
                            ));
                        }
                        if !seen.insert(id) {
                            problems.push(prob(&path, format!("lists pack \"{id}\" twice")));
                        }
                    }
                    profiles.insert(p.name.clone(), p);
                }
                Err(e) => problems.push(prob(&path, format!("invalid TOML: {e}"))),
            }
        }
        problems.extend(scalar_conflicts(&packs));
        if problems.is_empty() {
            Ok(Catalog { packs, profiles })
        } else {
            Err(problems)
        }
    }

    /// Packs in a profile, in the profile's order.
    #[cfg(test)]
    pub fn profile_packs(&self, name: &str) -> Option<Vec<&Pack>> {
        let p = self.profiles.get(name)?;
        Some(p.packs.iter().filter_map(|id| self.packs.get(id)).collect())
    }
}

fn prob(file: &str, message: impl Into<String>) -> Problem {
    Problem {
        file: file.to_string(),
        message: message.into(),
    }
}

fn load_pack(src: &dyn Source, dir: &str, problems: &mut Vec<Problem>) -> Option<Pack> {
    let mpath = format!("packs/{dir}/pack.toml");
    let Some(bytes) = src.read(&mpath) else {
        problems.push(prob(&mpath, "missing"));
        return None;
    };
    let manifest: Manifest = match toml::from_str(&String::from_utf8_lossy(&bytes)) {
        Ok(m) => m,
        Err(e) => {
            problems.push(prob(&mpath, format!("invalid TOML: {e}")));
            return None;
        }
    };
    let before = problems.len();
    if manifest.id != dir {
        problems.push(prob(
            &mpath,
            format!(
                "id \"{}\" must match the directory name \"{dir}\"",
                manifest.id
            ),
        ));
    }
    if !valid_id(&manifest.id) {
        problems.push(prob(
            &mpath,
            "id must be lowercase letters, digits and hyphens",
        ));
    }
    if crate::core::version::parse(&manifest.version).is_none() {
        problems.push(prob(
            &mpath,
            format!("version \"{}\" is not a dotted number", manifest.version),
        ));
    }
    if !valid_id(&manifest.category) {
        problems.push(prob(
            &mpath,
            "category must be lowercase letters, digits and hyphens",
        ));
    }
    for (field, value) in [
        ("title", &manifest.title),
        ("summary", &manifest.summary),
        ("limits", &manifest.limits),
    ] {
        if value.trim().is_empty() {
            problems.push(prob(&mpath, format!("{field} must not be empty")));
        }
    }
    if manifest.targets.is_empty() {
        problems.push(prob(&mpath, "declares no targets"));
    }

    let rpath = format!("packs/{dir}/rules.md");
    let rules = match src.read(&rpath) {
        Some(b) if !String::from_utf8_lossy(&b).trim().is_empty() => {
            String::from_utf8_lossy(&b).into_owned()
        }
        _ => {
            problems.push(prob(
                &rpath,
                "missing or empty; every pack states its rules for the agent",
            ));
            String::new()
        }
    };

    let mut targets = BTreeMap::new();
    for (target, spec) in &manifest.targets {
        if let Some(v) = &spec.min_version {
            if crate::core::version::parse(v).is_none() {
                problems.push(prob(
                    &mpath,
                    format!("targets.{target}.min_version \"{v}\" is not a dotted number"),
                ));
            }
        }
        // M1: advisory packs are instructions only. Writing settings or hooks into the
        // user's own configuration is invasive and needs its own design (merge, not replace).
        if manifest.tier == Tier::Advisory && (spec.settings.is_some() || !spec.hooks.is_empty()) {
            problems.push(prob(&mpath, format!("targets.{target}: advisory packs may only provide rules.md, not settings or hooks")));
        }
        let settings = match &spec.settings {
            None => None,
            Some(rel) => {
                let path = format!("packs/{dir}/{rel}");
                if !safe_relative(rel) {
                    problems.push(prob(
                        &mpath,
                        format!(
                            "targets.{target}.settings must be a relative path inside the pack"
                        ),
                    ));
                    None
                } else {
                    match src
                        .read(&path)
                        .map(|b| serde_json::from_slice::<serde_json::Value>(&b))
                    {
                        None => {
                            problems.push(prob(&path, "missing"));
                            None
                        }
                        Some(Err(e)) => {
                            problems.push(prob(&path, format!("invalid JSON: {e}")));
                            None
                        }
                        Some(Ok(v)) if !v.is_object() => {
                            problems.push(prob(&path, "must be a JSON object"));
                            None
                        }
                        Some(Ok(v)) => {
                            check_placeholders(&v, &path, problems);
                            Some(v)
                        }
                    }
                }
            }
        };
        let mut hooks = Vec::new();
        for rel in &spec.hooks {
            let path = format!("packs/{dir}/{rel}");
            if !safe_relative(rel) {
                problems.push(prob(
                    &mpath,
                    format!("hook \"{rel}\" must be a relative path inside the pack"),
                ));
                continue;
            }
            let name = Path::new(rel)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if hooks.iter().any(|h: &Hook| h.name == name) {
                problems.push(prob(&mpath, format!("two hooks are both named \"{name}\"")));
                continue;
            }
            match src.read(&path) {
                Some(content) => hooks.push(Hook { name, content }),
                None => problems.push(prob(&path, "missing")),
            }
        }
        targets.insert(
            target.clone(),
            TargetFiles {
                spec: spec.clone(),
                settings,
                hooks,
            },
        );
    }

    (problems.len() == before).then_some(Pack {
        manifest,
        rules,
        targets,
    })
}

fn check_placeholders(v: &serde_json::Value, path: &str, problems: &mut Vec<Problem>) {
    match v {
        serde_json::Value::String(s) => {
            let mut rest = s.as_str();
            while let Some(start) = rest.find('@') {
                let after = &rest[start + 1..];
                match after.find('@') {
                    Some(end)
                        if after[..end]
                            .chars()
                            .all(|c| c.is_ascii_uppercase() || c == '_')
                            && end > 0 =>
                    {
                        let token = &rest[start..start + end + 2];
                        if !PLACEHOLDERS.contains(&token) {
                            problems.push(prob(
                                path,
                                format!(
                                    "unknown placeholder {token} (known: {})",
                                    PLACEHOLDERS.join(", ")
                                ),
                            ));
                        }
                        rest = &after[end + 1..];
                    }
                    _ => break,
                }
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| check_placeholders(x, path, problems)),
        serde_json::Value::Object(o) => o
            .values()
            .for_each(|x| check_placeholders(x, path, problems)),
        _ => {}
    }
}

/// Two packs must not set the same scalar settings key to different values for one target.
///
/// Claude Code merges `managed-settings.d/` fragments in name order and the later scalar
/// wins, silently. Lists are unioned and nested objects merge by key, so only scalars clash.
fn scalar_conflicts(packs: &BTreeMap<String, Pack>) -> Vec<Problem> {
    let mut seen: BTreeMap<(String, String), (serde_json::Value, String)> = BTreeMap::new();
    let mut problems = Vec::new();
    for pack in packs.values() {
        for (target, files) in &pack.targets {
            let Some(settings) = &files.settings else {
                continue;
            };
            let mut scalars = Vec::new();
            collect_scalars(settings, String::new(), &mut scalars);
            for (key, value) in scalars {
                let k = (target.clone(), key.clone());
                match seen.get(&k) {
                    Some((other, owner)) if *other != value => problems.push(prob(
                        &format!("packs/{}/{target}/settings.json", pack.id()),
                        format!("sets {key} = {value}, but pack \"{owner}\" sets it to {other}; the later file would silently win"),
                    )),
                    Some(_) => {}
                    None => {
                        seen.insert(k, (value, pack.id().to_string()));
                    }
                }
            }
        }
    }
    problems
}

fn collect_scalars(
    v: &serde_json::Value,
    prefix: String,
    out: &mut Vec<(String, serde_json::Value)>,
) {
    match v {
        serde_json::Value::Object(o) => {
            for (k, x) in o {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                collect_scalars(x, p, out);
            }
        }
        serde_json::Value::Array(_) => {}
        other => out.push((prefix, other.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// An in-memory catalog for tests.
    pub struct MemSource(pub HashMap<String, Vec<u8>>);

    impl Source for MemSource {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.0.get(path).cloned()
        }
        fn entries(&self, dir: &str) -> Vec<String> {
            let prefix = format!("{dir}/");
            let mut v: Vec<String> = self
                .0
                .keys()
                .filter_map(|k| k.strip_prefix(&prefix))
                .map(|rest| rest.split('/').next().unwrap().to_string())
                .collect();
            v.sort();
            v.dedup();
            v
        }
    }

    fn pack(id: &str, settings: &str) -> Vec<(String, Vec<u8>)> {
        vec![
            (
                format!("packs/{id}/pack.toml"),
                format!(
                    "id = \"{id}\"\nversion = \"1.0.0\"\ncategory = \"security\"\ntier = \"enforced\"\n\
                     title = \"t\"\nsummary = \"s\"\nlimits = \"l\"\n\n[targets.claude-code]\n\
                     enforcement = \"enforced\"\nsettings = \"claude-code/settings.json\"\n"
                )
                .into_bytes(),
            ),
            (format!("packs/{id}/rules.md"), b"### Rules\n- one\n".to_vec()),
            (format!("packs/{id}/claude-code/settings.json"), settings.as_bytes().to_vec()),
        ]
    }

    fn mem(files: Vec<(String, Vec<u8>)>) -> MemSource {
        MemSource(files.into_iter().collect())
    }

    #[test]
    fn loads_a_valid_catalog() {
        let mut f = pack("a", r#"{"x": true}"#);
        f.extend(pack("b", r#"{"y": 1}"#));
        f.push((
            "profiles/base.toml".into(),
            b"name = \"base\"\ndescription = \"d\"\npacks = [\"a\", \"b\"]\n".to_vec(),
        ));
        let c = Catalog::load(&mem(f)).unwrap();
        assert_eq!(c.packs.len(), 2);
        assert_eq!(c.profile_packs("base").unwrap().len(), 2);
    }

    #[test]
    fn reports_every_problem_with_its_file() {
        let mut f = pack("a", r#"{"x": true}"#);
        f.extend(pack("b", "not json"));
        f.push((
            "profiles/base.toml".into(),
            b"name = \"wrong\"\ndescription = \"d\"\npacks = [\"nope\"]\n".to_vec(),
        ));
        let problems = Catalog::load(&mem(f)).unwrap_err();
        let text: Vec<String> = problems.iter().map(|p| p.to_string()).collect();
        assert!(
            text.iter()
                .any(|t| t.starts_with("packs/b/claude-code/settings.json: invalid JSON")),
            "{text:?}"
        );
        assert!(
            text.iter().any(|t| t.contains("must match the file name")),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|t| t.contains("\"nope\", which does not exist")),
            "{text:?}"
        );
    }

    #[test]
    fn rejects_conflicting_scalars_but_allows_lists() {
        let mut f = pack(
            "a",
            r#"{"cleanupPeriodDays": 7, "permissions": {"deny": ["X"]}}"#,
        );
        f.extend(pack(
            "b",
            r#"{"cleanupPeriodDays": 30, "permissions": {"deny": ["Y"]}}"#,
        ));
        let problems = Catalog::load(&mem(f)).unwrap_err();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("cleanupPeriodDays"));
    }

    #[test]
    fn rejects_unknown_placeholders_and_unknown_fields() {
        let mut f = pack("a", r#"{"hooks": {"x": "@HOOKS_DIR@/guard.sh"}}"#);
        f[0].1.extend_from_slice(b"typo_field = 1\n");
        let problems = Catalog::load(&mem(f)).unwrap_err();
        assert!(
            problems.iter().any(|p| p.message.contains("invalid TOML")),
            "{problems:?}"
        );
        let f2 = pack("b", r#"{"hooks": {"x": "@HOOKS_DIR@/guard.sh"}}"#);
        let problems = Catalog::load(&mem(f2)).unwrap_err();
        assert!(
            problems
                .iter()
                .any(|p| p.message.contains("unknown placeholder @HOOKS_DIR@")),
            "{problems:?}"
        );
    }

    #[test]
    fn advisory_packs_may_not_ship_settings() {
        let mut f = pack("a", r#"{"x": true}"#);
        let manifest = String::from_utf8(f[0].1.clone())
            .unwrap()
            .replace("tier = \"enforced\"", "tier = \"advisory\"");
        f[0].1 = manifest.into_bytes();
        let problems = Catalog::load(&mem(f)).unwrap_err();
        assert!(problems.iter().any(|p| p
            .message
            .contains("advisory packs may only provide rules.md")));
    }

    #[test]
    fn the_shipped_catalog_is_valid() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("catalog");
        let c = Catalog::load(&DirSource(root)).unwrap_or_else(|p| panic!("{p:#?}"));
        assert_eq!(c.packs.len(), 8);
        assert_eq!(c.profiles.len(), 3);
    }
}
