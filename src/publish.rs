//! `handrail publish <pack> --catalog <dir>`: propose a pack to a shared catalog.
//!
//! Publishing is a pull request, never a direct write: a pack can end up running as root
//! on other people's machines, so a person reviews it before anyone installs it by
//! default. For sharing without review, a pack in any git repository already works:
//! `handrail enable <pack> --catalog github.com/you/your-packs`.
//!
//! Everything goes through the GitHub REST API, over one of two transports:
//!
//! - `gh api`, when the GitHub CLI is installed and signed in;
//! - `curl` with a token from `GITHUB_TOKEN` or `GH_TOKEN` (scope: `public_repo`).
//!
//! Neither needs git, a local clone, or a fork made by hand: if you cannot push to the
//! catalog repository, a fork is created in your account and the pull request comes
//! from there. With neither transport available, the manual steps are printed instead.

use crate::context::Ctx;
use crate::core::catalog::{Origin, Pack};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The shared catalog. `--repo` points elsewhere, e.g. a team's own packs repository.
pub const DEFAULT_REPO: &str = "martinx/handrail-packs";

pub struct Opts {
    pub repo: String,
    pub dry_run: bool,
    pub yes: bool,
}

pub fn publish(ctx: &Ctx, id: &str, opts: &Opts) -> Result<(), String> {
    let pack = ctx.catalog.packs.get(id).ok_or_else(|| {
        format!(
            "no pack named \"{id}\". Point at your catalog: handrail publish {id} --catalog <dir>"
        )
    })?;
    let dir = source_dir(ctx, pack)?;

    println!("Checking the catalog before publishing:\n");
    if crate::check::check(&dir) > 0 {
        return Err("Fix the problems above first; nothing was published.".into());
    }

    let files = pack_files(pack);
    let branch = format!("pack/{id}-{}", pack.manifest.version);
    println!(
        "\nPublish {id} v{} to {} as a pull request (branch {branch}):",
        pack.manifest.version, opts.repo
    );
    for (path, _, _) in &files {
        println!("  {path}");
    }
    println!(
        "\nThese files become public on GitHub. The pull request is reviewed before anyone\ninstalls the pack by default."
    );

    let api = match Api::detect() {
        Some(api) => api,
        None => {
            manual_steps(id, &dir, &opts.repo);
            return if opts.dry_run {
                Ok(())
            } else {
                Err("No GitHub access: install and sign in to gh, or set GITHUB_TOKEN.".into())
            };
        }
    };
    println!("Using {}.", api.describe());
    if opts.dry_run {
        println!("\nDry run: nothing was sent.");
        return Ok(());
    }
    crate::change::confirm(opts.yes)?;

    let url = open_pull_request(&api, pack, &files, &branch, &opts.repo)?;
    println!("\nPull request: {url}");
    println!(
        "Until it is merged, anyone can use the pack from your branch or repository with:\n  handrail enable {id} --catalog <git URL of a repository containing packs/{id}>"
    );
    Ok(())
}

/// The directory the pack was loaded from. Publishing an installed copy or a temporary
/// clone would publish something other than what the author is working on.
fn source_dir(ctx: &Ctx, pack: &Pack) -> Result<PathBuf, String> {
    let id = pack.id();
    let Origin::External(label) = &pack.origin else {
        return Err(format!(
            "\"{id}\" is a built-in pack. To change it, open a pull request on {DEFAULT_REPO}."
        ));
    };
    ctx.catalogs
        .iter()
        .find(|c| &c.label == label && !c.is_clone())
        .map(|c| c.dir.clone())
        .ok_or_else(|| {
            format!(
                "\"{id}\" comes from {label}, not from a local directory. Publish from the directory you edit it in:\n  handrail publish {id} --catalog <dir>"
            )
        })
}

/// (path in the catalog repository, contents, git file mode)
fn pack_files(pack: &Pack) -> Vec<(String, Vec<u8>, &'static str)> {
    pack.files
        .iter()
        .map(|(rel, bytes)| {
            let mode = if rel.ends_with(".sh") {
                "100755"
            } else {
                "100644"
            };
            (format!("packs/{}/{rel}", pack.id()), bytes.clone(), mode)
        })
        .collect()
}

fn manual_steps(id: &str, dir: &Path, repo: &str) {
    println!(
        "\nNo GitHub access found (neither a signed-in gh nor GITHUB_TOKEN). To publish by hand:\n\n  1. Fork https://github.com/{repo}\n  2. Copy {}/packs/{id} into packs/{id} of your fork, on a new branch\n  3. Open a pull request against {repo}\n\nOr let handrail do it: install gh (https://cli.github.com) and run `gh auth login`,\nor set GITHUB_TOKEN to a token with the public_repo scope.",
        dir.display()
    );
}

fn open_pull_request(
    api: &Api,
    pack: &Pack,
    files: &[(String, Vec<u8>, &'static str)],
    branch: &str,
    upstream: &str,
) -> Result<String, String> {
    let id = pack.id();
    let login = str_field(&api.call("GET", "/user", None)?, "login")?;
    let up = api.call("GET", &format!("/repos/{upstream}"), None)?;
    let base = str_field(&up, "default_branch")?;
    let can_push = up["permissions"]["push"].as_bool().unwrap_or(false);

    // Where the branch goes: the catalog itself if you may push to it, else your fork
    let head_repo = if can_push {
        upstream.to_string()
    } else {
        println!("Forking {upstream} to your account (or reusing your fork)…");
        let fork = api.call("POST", &format!("/repos/{upstream}/forks"), Some(json!({})))?;
        let name = str_field(&fork, "full_name")?;
        wait_for(api, &name)?;
        name
    };

    let base_sha = str_field(
        &api.call(
            "GET",
            &format!("/repos/{upstream}/git/ref/heads/{base}"),
            None,
        )?["object"],
        "sha",
    )?;
    if head_repo != upstream {
        // Bring the fork's default branch up to date so the base commit exists in it
        let _ = api.call(
            "POST",
            &format!("/repos/{head_repo}/merge-upstream"),
            Some(json!({ "branch": base })),
        );
    }
    let base_tree = str_field(
        &api.call(
            "GET",
            &format!("/repos/{head_repo}/git/commits/{base_sha}"),
            None,
        )?["tree"],
        "sha",
    )?;

    // Files of an earlier version of the pack that this version no longer has
    let prefix = format!("packs/{id}/");
    let existing = api.call(
        "GET",
        &format!("/repos/{head_repo}/git/trees/{base_tree}?recursive=1"),
        None,
    )?;
    let updating = existing["tree"].as_array().is_some_and(|t| {
        t.iter()
            .any(|e| e["path"].as_str() == Some(&format!("packs/{id}")))
    });
    let mut tree: Vec<Value> = Vec::new();
    for e in existing["tree"].as_array().into_iter().flatten() {
        let (Some(path), Some("blob")) = (e["path"].as_str(), e["type"].as_str()) else {
            continue;
        };
        if path.starts_with(&prefix) && !files.iter().any(|(p, _, _)| p == path) {
            tree.push(json!({ "path": path, "mode": "100644", "type": "blob", "sha": null }));
        }
    }
    for (path, bytes, mode) in files {
        let blob = api.call(
            "POST",
            &format!("/repos/{head_repo}/git/blobs"),
            Some(json!({ "content": base64(bytes), "encoding": "base64" })),
        )?;
        tree.push(
            json!({ "path": path, "mode": mode, "type": "blob", "sha": str_field(&blob, "sha")? }),
        );
    }
    let new_tree = api.call(
        "POST",
        &format!("/repos/{head_repo}/git/trees"),
        Some(json!({ "base_tree": base_tree, "tree": tree })),
    )?;
    let title = format!(
        "{} {id} v{}",
        if updating { "Update" } else { "Add" },
        pack.manifest.version
    );
    let commit = api.call(
        "POST",
        &format!("/repos/{head_repo}/git/commits"),
        Some(json!({
            "message": title,
            "tree": str_field(&new_tree, "sha")?,
            "parents": [base_sha],
        })),
    )?;
    let commit_sha = str_field(&commit, "sha")?;

    // Create the branch, or move it if this is a re-publish of the same version
    let created = api.try_call(
        "POST",
        &format!("/repos/{head_repo}/git/refs"),
        Some(json!({ "ref": format!("refs/heads/{branch}"), "sha": commit_sha })),
    )?;
    if created.0 == 422 {
        api.call(
            "PATCH",
            &format!("/repos/{head_repo}/git/refs/heads/{branch}"),
            Some(json!({ "sha": commit_sha, "force": true })),
        )?;
    }

    let head = if head_repo == upstream {
        branch.to_string()
    } else {
        format!("{login}:{branch}")
    };
    let (status, pr) = api.try_call(
        "POST",
        &format!("/repos/{upstream}/pulls"),
        Some(json!({
            "title": title,
            "head": head,
            "base": base,
            "body": pr_body(pack),
            "maintainer_can_modify": true,
        })),
    )?;
    if status == 201 {
        return str_field(&pr, "html_url");
    }
    // A pull request for this branch is already open: the push above updated it
    let owner = head_repo.split('/').next().unwrap_or(&login);
    let open = api.call(
        "GET",
        &format!("/repos/{upstream}/pulls?head={owner}:{branch}&state=open"),
        None,
    )?;
    match open.as_array().and_then(|a| a.first()) {
        Some(pr) => {
            println!("A pull request for this version was already open; it now has these files.");
            str_field(pr, "html_url")
        }
        None => Err(format!(
            "creating the pull request failed (HTTP {status}): {pr}"
        )),
    }
}

fn pr_body(pack: &Pack) -> String {
    let m = &pack.manifest;
    let mut s = format!(
        "**{}** — {}\n\n- id: `{}`\n- version: {}\n- category: {}\n- tier: {:?}\n",
        m.title, m.summary, m.id, m.version, m.category, m.tier
    );
    for (t, f) in &pack.targets {
        s.push_str(&format!("- {t}: {}\n", f.spec.enforcement.as_str()));
    }
    if !m.protects.is_empty() {
        s.push_str("\nProtects against:\n");
        m.protects
            .iter()
            .for_each(|x| s.push_str(&format!("- {x}\n")));
    }
    if !m.tradeoffs.is_empty() {
        s.push_str("\nTradeoffs:\n");
        m.tradeoffs
            .iter()
            .for_each(|x| s.push_str(&format!("- {x}\n")));
    }
    s.push_str(&format!("\nLimits: {}\n", m.limits));
    s.push_str(&format!(
        "\n`handrail check` passed locally (handrail {}).\n\n_Opened with `handrail publish`._\n",
        crate::change::VERSION
    ));
    s
}

/// Forks are created asynchronously; wait until the fork answers.
fn wait_for(api: &Api, repo: &str) -> Result<(), String> {
    for _ in 0..30 {
        if api.try_call("GET", &format!("/repos/{repo}"), None)?.0 == 200 {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    Err(format!(
        "the fork {repo} did not become ready in time; run publish again"
    ))
}

fn str_field(v: &Value, key: &str) -> Result<String, String> {
    v[key]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("unexpected GitHub response (no \"{key}\"): {v}"))
}

/// How requests reach GitHub.
enum Api {
    Gh,
    Token { token: String, base: String },
}

impl Api {
    fn detect() -> Option<Api> {
        let base = std::env::var("HANDRAIL_GITHUB_API")
            .unwrap_or_else(|_| "https://api.github.com".into());
        if let Some(token) = ["GITHUB_TOKEN", "GH_TOKEN"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|t| !t.is_empty()))
        {
            return Some(Api::Token { token, base });
        }
        let gh_ready = Command::new("gh")
            .args(["auth", "status"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        gh_ready.then_some(Api::Gh)
    }

    fn describe(&self) -> &'static str {
        match self {
            Api::Gh => "the GitHub CLI (gh)",
            Api::Token { .. } => "the token in GITHUB_TOKEN / GH_TOKEN",
        }
    }

    /// A request that must succeed (2xx).
    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        let (status, v) = self.try_call(method, path, body)?;
        if (200..300).contains(&status) {
            Ok(v)
        } else {
            Err(format!(
                "GitHub: {method} {path} failed (HTTP {status}): {v}"
            ))
        }
    }

    /// A request whose status the caller inspects.
    fn try_call(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<(u16, Value), String> {
        let mut cmd = match self {
            Api::Gh => {
                let mut c = Command::new("gh");
                c.args([
                    "api",
                    "--include",
                    "--method",
                    method,
                    path.trim_start_matches('/'),
                ]);
                if body.is_some() {
                    c.args(["--input", "-"]);
                }
                c
            }
            Api::Token { base, .. } => {
                let mut c = Command::new("curl");
                c.args(["-sS", "--include", "-X", method])
                    .args(["-H", "Accept: application/vnd.github+json"])
                    .args(["-H", "X-GitHub-Api-Version: 2022-11-28"])
                    .args(["-H", "User-Agent: handrail"])
                    // The token goes in through stdin config, not argv, which other users can see
                    .args(["--config", "-"])
                    .arg(format!("{base}{path}"));
                if body.is_some() {
                    c.args(["-H", "Content-Type: application/json"]);
                }
                c
            }
        };
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not run {}: {e}", self.program()))?;
        {
            let mut stdin = child.stdin.take().expect("piped");
            match self {
                Api::Gh => {
                    if let Some(b) = &body {
                        stdin.write_all(b.to_string().as_bytes()).ok();
                    }
                }
                Api::Token { token, .. } => {
                    let mut cfg = format!("header = \"Authorization: Bearer {token}\"\n");
                    if let Some(b) = &body {
                        cfg.push_str(&format!("data-binary = {}\n", curl_quote(&b.to_string())));
                    }
                    stdin.write_all(cfg.as_bytes()).ok();
                }
            }
        }
        let out = child
            .wait_with_output()
            .map_err(|e| format!("{} failed: {e}", self.program()))?;
        let text = String::from_utf8_lossy(&out.stdout);
        match parse_response(&text) {
            Some(r) => Ok(r),
            None => Err(format!(
                "{} {method} {path}: no HTTP response\n{}",
                self.program(),
                String::from_utf8_lossy(&out.stderr).trim()
            )),
        }
    }

    fn program(&self) -> &'static str {
        match self {
            Api::Gh => "gh",
            Api::Token { .. } => "curl",
        }
    }
}

/// A string for curl's config file: double-quoted, with `\` and `"` escaped.
fn curl_quote(s: &str) -> String {
    let mut q = String::with_capacity(s.len() + 2);
    q.push('"');
    for c in s.chars() {
        match c {
            '\\' => q.push_str("\\\\"),
            '"' => q.push_str("\\\""),
            '\n' => q.push_str("\\n"),
            '\r' => q.push_str("\\r"),
            '\t' => q.push_str("\\t"),
            c => q.push(c),
        }
    }
    q.push('"');
    q
}

/// Status and JSON body from `--include` output: status line, headers, blank line, body.
/// Interim responses (`100 Continue`) come first and are skipped.
fn parse_response(text: &str) -> Option<(u16, Value)> {
    let mut rest = text;
    loop {
        let start = rest.find("HTTP/")?;
        rest = &rest[start..];
        let status: u16 = rest.split_whitespace().nth(1)?.parse().ok()?;
        let body_at = rest
            .find("\r\n\r\n")
            .map(|i| i + 4)
            .or_else(|| rest.find("\n\n").map(|i| i + 2))
            .unwrap_or(rest.len());
        let body = &rest[body_at..];
        if (100..200).contains(&status) {
            rest = body;
            continue;
        }
        let v = serde_json::from_str(body.trim()).unwrap_or(Value::Null);
        return Some((status, v));
    }
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn responses_parse_with_interim_status_and_either_line_ending() {
        let t = "HTTP/1.1 100 Continue\r\n\r\nHTTP/2 201 Created\r\nx: y\r\n\r\n{\"sha\":\"abc\"}";
        assert_eq!(parse_response(t), Some((201, json!({"sha": "abc"}))));
        let g = "HTTP/2.0 422 Unprocessable Entity\nContent-Type: json\n\n{\"message\":\"exists\"}";
        assert_eq!(parse_response(g).unwrap().0, 422);
        assert_eq!(parse_response("curl: (6) could not resolve"), None);
    }

    #[test]
    fn curl_config_strings_are_escaped() {
        assert_eq!(curl_quote(r#"{"a":"b\"c"}"#), r#""{\"a\":\"b\\\"c\"}""#);
        assert_eq!(curl_quote("x\ny"), r#""x\ny""#);
    }
}
