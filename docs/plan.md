# Development plan

> Companion to [design.md](design.md). Sizes are rough (S ≈ ½ day, M ≈ 1–2 days, L ≈ 3+ days).
> "Blocked on" names the external step a milestone waits for.

## Where we are

**M1 steps 1–8 done** (2026-09-22): the Rust CLI replaces the shell prototype, which has
been removed. 44 tests. Remaining for the M1 exit criteria: CI on real macOS and Linux runners
(needs the repository on GitHub) and a first install on a real machine with `sudo`.

## M0 (done)

- POSIX-sh installer for Claude Code: 8 security modules, 3 profiles, install / uninstall /
  status / dry-run / local rules. 58 tests, all in a temporary directory.
- Design, name (Handrail), license (Apache-2.0).

## Now — M0.5: reserve and dogfood (in parallel with M1)

| Task | Who | Size | Notes |
|---|---|---|---|
| Register `handrail.sh` (optionally `gethandrail.dev`) | maintainer | S | The only resource someone else could take with no good substitute. Until then the site uses `handrail.bitey.ai` |
| Point `handrail.bitey.ai` at GitHub Pages: verify `bitey.ai` in GitHub first, then add `handrail CNAME martinx.github.io.` | maintainer | S | Verification first prevents subdomain takeover while the CNAME points at GitHub before our Pages site exists |
| ~~Reserve the `handrail` crate~~ | done | — | 0.0.1 published 2026-09-22, owner `martinx`, tag `v0.0.1`. Later releases use crates.io Trusted Publishing from GitHub Actions (no long-lived token) |
| Install M0 `baseline` on the maintainer's machine and use it daily | maintainer | S | Real use surfaces friction before anyone else sees it |
| Decide when the repository goes public | maintainer | — | homebrew-core counts repository age (30 days minimum), so an earlier public repo starts the clock earlier |

## M1 — Rust CLI, Claude Code only

Goal: the `handrail` binary replaces `install.sh` with feature parity, plus the two tiers
and transactional apply. No network yet: packs are embedded in the binary.

| # | Task | Size | Done when |
|---|---|---|---|
| 1 | Workspace: `handrail-core` (pack model, planner), `handrail-claude` (adapter), `handrail` (CLI); CI workflow (fmt, clippy `-D warnings`, tests) | S | CI is green on an empty skeleton |
| 2 | Pack format v1: `pack.toml` schema, loader, validation with actionable errors | M | Invalid packs fail with the file, the field, and why |
| 3 | Migrate the 8 modules to packs (category, tier, per-target enforcement level, tradeoffs, limits, test vectors) | S | All 8 load and validate |
| 4 | Planner: desired set → per-tier file set; diff against installed state; scalar-key conflict check; `min_version` gating against `claude --version` | M | `handrail plan` prints exactly what `apply` would write |
| 5 | Claude Code adapter: managed paths (macOS, Linux), `managed-settings.d/` fragments, managed `CLAUDE.md` block, hooks, advisory tier in `~/.claude/rules/`; detect MDM policy that would override file-based policy | M | Same files as M0 for the same selection |
| 6 | Transactional apply with privilege separation: plan unprivileged, write a plan file, `sudo handrail apply --plan <file>` verifies its hash, stages, validates every file, swaps atomically, keeps a rollback snapshot; `handrail rollback` | L | Killing the process at any point leaves either the old state or the new one, never a mix |
| 7 | Commands: `list`, `show`, `enable`, `disable`, `profile use`, `plan`, `apply`, `status`, `rollback`, `doctor`, `rule add` | M | `--help` for every command; enforced changes always go through step 6 |
| 8 | Port the 58 M0 tests to Rust integration tests; keep the hook test-vector runner | M | Parity with M0, plus tests for steps 4 and 6 |

**Exit criteria:** feature parity with M0; enforced packs cannot be changed without
administrator rights; idempotent; nothing left behind on `disable --all`; macOS arm64/x64
and Linux x64 pass CI.

## M2 — Distribution, remote registry, UI

**Blocked on:** a public repository (GitHub Pages on a free plan needs one), a signing key. The site starts on `handrail.bitey.ai`; `handrail.sh` replaces it once registered.

| # | Task | Size |
|---|---|---|
| 1 | Release workflow: build matrix, checksums, minisign signatures, GitHub Release, crates.io publish, `martinx/homebrew-tap` formula update | M |
| 2 | Registry repository layout (packs + `index.json` + signature); `handrail update`: check, diff, approve; pinned versions; `--registry <url|path>` mirrors | L |
| 3 | `curl -fsSL https://handrail.bitey.ai/install.sh \| sh` (later `handrail.sh`): bootstrap that downloads a pinned binary and verifies checksum and signature | S |
| 4 | Terminal UI (`handrail ui`): categories, tier badges, enforcement matrix, toggles; enforced toggles go through the admin prompt | L |
| 5 | Website on `handrail.bitey.ai` (later `handrail.sh`): landing page, catalog generated from the registry, configurator that outputs the exact command. Static; no third-party requests | M |
| 6 | Contributing: `CONTRIBUTING.md`, pack template, PR validation in CI, CODEOWNERS requiring two maintainers for packs with executable hooks | M |

**Exit criteria:** a user on a clean Mac goes from the website to an installed, verified
baseline in under two minutes, and every downloaded artifact is signature-checked.

## M3 — Beyond security

| # | Task | Size |
|---|---|---|
| 1 | Advisory tier end to end (user scope, free toggling, optional auto-apply) | M |
| 2 | Seed categories with 3–5 packs each: communication, git hygiene, code quality, testing, cost | L |
| 3 | User-defined profiles; export/import a profile file to share with a team | M |
| 4 | Translation overlays for human-facing text | S |

## M4 — Second agent

Chosen by user demand after M2. For that agent: document its configuration and enforcement
surface from its own documentation, implement the adapter, publish the enforcement matrix
(enforced / partial / advisory / unsupported per pack). Size L per agent.

## Carried into M2

- The published crate must contain the catalog: `include_dir!` currently reads
  `../../catalog`, which `cargo publish` does not package. Move the catalog into the crate or
  generate it at build time before the next crates.io release.

## Standing work

- Weekly CI run against the latest Claude Code release: settings keys appear and change often.
- Every pack keeps its `limits` honest. A pack that claims more than it enforces is a bug.
