//! `handrail check <dir>`: everything a pack must pass, for pack authors and for the
//! handrail-packs repository's CI. The same checks run in this repository's tests.

use crate::core::catalog::{Catalog, DirSource};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Runs every check and prints a report. Returns the number of failures.
pub fn check(dir: &Path) -> usize {
    let mut failures = 0;
    println!("Checking {}", dir.display());

    match Catalog::load(&DirSource(dir.to_path_buf())) {
        Ok(c) => println!(
            "  ok   catalog: {} packs, {} profiles",
            c.packs.len(),
            c.profiles.len()
        ),
        Err(problems) => {
            for p in &problems {
                println!("  FAIL {p}");
            }
            failures += problems.len();
        }
    }

    let vectors = hook_vectors(&dir.join("packs"));
    let failed: Vec<&Vector> = vectors.iter().filter(|v| !v.passed()).collect();
    for v in &failed {
        println!(
            "  FAIL {}: {} (expected exit {}, got {})",
            v.hook.display(),
            v.name,
            v.want,
            v.got.map_or("none".into(), |c| c.to_string())
        );
    }
    println!(
        "  {}   hook test vectors: {} of {} passed",
        if failed.is_empty() { "ok" } else { "FAIL" },
        vectors.len() - failed.len(),
        vectors.len()
    );
    failures += failed.len();

    let lint = lint_scripts(dir);
    for l in &lint {
        println!("  FAIL {l}");
    }
    if lint.is_empty() {
        println!("  ok   shell scripts: no $VAR directly followed by non-ASCII text");
    }
    failures += lint.len();

    println!(
        "{}",
        if failures == 0 {
            "All checks passed."
        } else {
            "Some checks failed."
        }
    );
    failures
}

/// One line of a pack's `tests/*.cases` file, and what running it produced.
pub struct Vector {
    pub hook: PathBuf,
    pub name: String,
    pub want: i32,
    pub got: Option<i32>,
}

impl Vector {
    pub fn passed(&self) -> bool {
        self.got == Some(self.want)
    }
}

/// Runs every `tests/<hook>.cases` against `<target>/hooks/<hook>.sh` in each pack.
///
/// A `.cases` line is tab-separated: name, expected exit code, JSON given on stdin.
/// Each case gets its own empty HOME, so hooks that write files cannot affect each other.
pub fn hook_vectors(packs: &Path) -> Vec<Vector> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(packs) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for pack in dirs {
        let Ok(cases) = std::fs::read_dir(pack.join("tests")) else {
            continue;
        };
        for case_file in cases.filter_map(|e| e.ok().map(|e| e.path())) {
            if case_file.extension().is_none_or(|e| e != "cases") {
                continue;
            }
            let stem = case_file
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let Some(hook) = find_hook(&pack, &stem) else {
                out.push(Vector {
                    hook: case_file.clone(),
                    name: format!("no hook named {stem}.sh for these cases"),
                    want: 0,
                    got: None,
                });
                continue;
            };
            for line in std::fs::read_to_string(&case_file)
                .unwrap_or_default()
                .lines()
            {
                if line.trim().is_empty() || line.starts_with('#') {
                    continue;
                }
                let mut parts = line.splitn(3, '\t');
                let name = parts.next().unwrap_or_default().to_string();
                let want = parts
                    .next()
                    .and_then(|w| w.trim().parse().ok())
                    .unwrap_or(-1);
                let input = parts.next().unwrap_or("");
                out.push(Vector {
                    got: run_hook(&hook, input),
                    hook: hook.clone(),
                    name,
                    want,
                });
            }
        }
    }
    out
}

fn find_hook(pack: &Path, stem: &str) -> Option<PathBuf> {
    let rd = std::fs::read_dir(pack).ok()?;
    rd.filter_map(|e| {
        e.ok()
            .map(|e| e.path().join("hooks").join(format!("{stem}.sh")))
    })
    .find(|p| p.exists())
}

fn run_hook(hook: &Path, input: &str) -> Option<i32> {
    let home = std::env::temp_dir().join(format!(
        "handrail-check-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::create_dir_all(&home).ok()?;
    let mut child = Command::new("sh")
        .arg(hook)
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(input.as_bytes()).ok()?;
    let code = child.wait().ok()?.code();
    let _ = std::fs::remove_dir_all(&home);
    code
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// sh swallows the leading bytes of a multibyte character into a variable name:
/// "$VERSION）" reads a variable named VERSION\xef\xbc and aborts under `set -u`.
/// Hooks are shell scripts, so every `*.sh` is checked. Use `${VAR}` before such text.
pub fn lint_scripts(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect(dir, &mut files);
    let mut out = Vec::new();
    for f in files
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "sh"))
    {
        let Ok(s) = std::fs::read_to_string(f) else {
            continue;
        };
        for (n, line) in s.lines().enumerate() {
            let b = line.as_bytes();
            for i in 0..b.len() {
                if b[i] != b'$' {
                    continue;
                }
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                if j > i + 1 && j < b.len() && b[j] >= 0x80 {
                    out.push(format!(
                        "{}:{}: use ${{VAR}} before non-ASCII text",
                        f.display(),
                        n + 1
                    ));
                }
            }
        }
    }
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if p.is_dir() {
            collect(&p, out);
        } else {
            out.push(p);
        }
    }
}
