# Design: a policy manager for coding agents

> Status: **draft for discussion**. Project name: **Handrail** (CLI: `handrail`).
> The shell prototype in this repo (M0) is the reference implementation for the Claude Code target.

## 1. What this is

A tool that lets people **browse, enable, disable and update rule packs** for the coding agents
they run locally — Claude Code first, other agents later — from one catalog, with one command
or a simple UI.

Security is one category, not the whole product. Others: communication, git hygiene, code
quality, testing discipline, cost control, team conventions, and whatever the community writes.

### Why this is worth building, and where the value actually is

"A collection of agent rules" already exists in many forms (prompt collections, rules files in
repos). Collections alone are not a product. What is missing everywhere:

1. **Honesty about enforcement.** A line in an instructions file is a *request* to the model.
   A setting evaluated by the agent's harness, a hook, or an OS sandbox is *enforcement*.
   Users today cannot tell which of their "rules" are real. We make that the first thing they see.
2. **Security-first defaults** that are actually enforced where the agent supports it.
3. **Local-first.** No account, no telemetry, no server. Updates are downloads, never uploads.
4. **One intent, many agents.** A pack describes *what* it wants ("never publish content to a
   hosted service"); each agent adapter compiles that to the strongest mechanism that agent has,
   and reports honestly when the best it can do is a sentence in an instructions file.

## 2. Core concepts

| Concept | Meaning |
|---|---|
| **Pack** | The unit users enable or disable. One intent, a category, a tier, per-target implementations, tradeoffs, limits, tests. |
| **Category** | Grouping for browsing: `security`, `privacy`, `communication`, `git`, `quality`, `cost`, … Open-ended. |
| **Tier** | `enforced` or `advisory`. Decides where a pack is installed and who can toggle it. See §3. |
| **Target** | An agent adapter (`claude-code`, later `codex`, `gemini-cli`, …). Knows where that agent reads configuration and which mechanisms exist. |
| **Profile** | A named set of packs (`baseline`, `strict`, `paranoid`, or user-defined). |
| **Registry** | A signed index of packs and versions, hosted in a git repo, mirrorable, usable offline. |

## 3. Two tiers, and why they must stay separate

The agent runs **as the user**. Anything the user can change without administrator
authentication, the agent can change too — including "disable this rule".

| | `enforced` | `advisory` |
|---|---|---|
| Purpose | Security and privacy guarantees | Preferences and conventions |
| Claude Code location | System-managed directory (`/Library/Application Support/ClaudeCode/`, `/etc/claude-code/`): `managed-settings.d/*.json`, managed `CLAUDE.md`, hooks | User scope: `~/.claude/rules/*.md`, `~/.claude/CLAUDE.md` block |
| Owner | root | the user |
| Toggle requires | OS administrator authentication | nothing |
| Agent can undo it | **No** | Yes (by design) |
| Auto-update | Never auto-applied — show diff, require approval | May auto-apply (opt-in) |

**Rule for the UI:** an enforced pack is never "one click off". Disabling it goes through an
OS admin prompt. If it didn't, the agent could turn off its own guardrails.

## 4. Pack format

```
packs/security/no-hosted-publishing/
  pack.toml          # metadata, tier, targets, tradeoffs, limits
  rules.md           # agent-facing instructions (English source of truth)
  targets/
    claude-code/
      settings.json  # merged into managed-settings.d/ (enforced) — may reference @HOOKS@
      hooks/*.sh     # optional; executable content, reviewed more strictly
  tests/
    *.cases          # hook test vectors: name, expected exit code, input
  i18n/
    zh-CN.toml       # optional translated title/summary/tradeoffs
```

```toml
id          = "no-hosted-publishing"
version     = "1.2.0"
category    = "privacy"
tier        = "enforced"
title       = "Keep everything on this machine"
summary     = "Turn off every optional upload: remote control, cloud agents, hosted artifacts, connectors, telemetry, feedback uploads."
tradeoffs   = ["No remote control from your phone", "/feedback, /bug and /share are unavailable"]
limits      = "Model inference itself sends the conversation to the model API. No local setting can change that."

[targets.claude-code]
enforcement  = "enforced"          # enforced | partial | advisory | unsupported
min_version  = "2.1.242"           # keys this pack relies on appeared in this version
settings     = "targets/claude-code/settings.json"
hooks        = ["targets/claude-code/hooks/guard.sh"]
```

`enforcement` is **per target**. The same pack can be `enforced` on Claude Code and
`advisory` on an agent that only reads an instructions file. The UI shows this matrix.

## 5. Targets (agent adapters)

An adapter answers four questions for its agent:

1. Where does it read configuration, at which precedence, and which location can the agent itself not write?
2. Which enforcement mechanisms exist (settings, permission rules, hooks, OS sandbox) and from which version?
3. How is an instructions file loaded (always, on demand, excludable by the user)?
4. How do we verify the policy is in force (e.g. `claude doctor`, `/status` for Claude Code)?

**v1 ships only `claude-code`**, the target with the richest enforcement surface: managed
settings (top precedence, `managed-settings.d/` fragments merged by documented rules), permission
rules, PreToolUse/PostToolUse hooks, an OS-level sandbox, and a managed `CLAUDE.md` that users
cannot exclude.

Later targets are added one at a time, each verified against that agent's own documentation
before we claim anything is "enforced". Where an agent only supports instruction files, packs
compile to that file (a shared format such as `AGENTS.md` where the agent reads it) and are
labelled `advisory` — never presented as enforcement.

## 6. Registry, updates and trust

The tool installs policy as root and may install executable hooks. **That makes the tool
itself a supply-chain target**: whoever can change the registry can run code as root on every
machine that auto-updates. Therefore:

- **Signed index.** Releases are signed (minisign or Sigstore); the public key is compiled into
  the binary. Unsigned or mismatched content is rejected.
- **Pinned versions.** The installed state records exact pack versions and content hashes.
- **Update policy.** `check` can run automatically. `apply` for enforced packs always shows a
  diff and requires admin approval. Packs containing executable hooks are **never** auto-applied.
- **Mirrors and offline.** Any git host or a local directory can serve the registry
  (`--registry <url|path>`); an offline bundle is a signed tarball.
- **No telemetry, ever.** Update checks are plain downloads of a public index.

### Contributions

PR to the registry repo → CI validates the schema, runs every pack's test vectors, lints
scripts (including the `$VAR`-followed-by-non-ASCII check) and installs into a throwaway
directory → review. Packs with executable hooks need two maintainer approvals. Maintainers
sign releases; contributors never need signing keys.

## 7. User interface

| Version | Surface | Why |
|---|---|---|
| v1 | CLI: `list`, `enable`, `disable`, `status`, `diff`, `update`, `doctor` | Scriptable; no listening socket |
| v1 | Terminal UI (checklist grouped by category, tier badges, enforcement matrix) | "Simple UI" with zero network attack surface |
| v1 | Static website: catalog + configurator that outputs the exact command | Discovery without running anything |
| later | Local desktop/web app | Only if demand justifies it. A local web server that can change root policy must bind 127.0.0.1, require a per-session token, check the Host header (DNS rebinding), and still hand enforced changes to an OS admin prompt |

## 8. Implementation

- **Single static binary in Rust.** No runtime dependencies on user machines; straightforward
  signature verification; a real TUI; testable. The POSIX-sh installer remains only as a
  bootstrap that downloads and verifies the binary.
- **Privilege separation.** Runs unprivileged. Writing the enforced tier re-executes a minimal
  privileged step via `sudo` (or the platform's admin prompt) with a precomputed, validated plan.
- **Transactional apply.** Build the full new state in a staging directory, validate every
  file (a managed settings file that isn't valid JSON makes Claude Code refuse to start), then
  swap atomically and keep a rollback snapshot. *The M0 prototype once stopped halfway — settings
  written, instructions file not — which is exactly what this rule prevents.*
- **Version awareness.** Detect the installed agent version; skip or warn on packs whose
  `min_version` is not met, instead of silently writing keys the agent ignores.
- **Conflict detection.** Two packs setting the same scalar key to different values is a
  build-time error (later file silently wins at runtime otherwise). Detect other managed
  sources (MDM, server-managed) that would make file-based policy ineffective and say so.

## 9. Distribution

| Channel | Command | When |
|---|---|---|
| Own Homebrew tap | `brew install martinx/tap/handrail` | From the first release. After `brew tap martinx/tap`, `brew install handrail` also works as long as homebrew-core has no formula of that name (it has none today) |
| homebrew-core | `brew install handrail` | Once eligible. Homebrew's Package Acceptance Policy: at least 30 forks, 30 watchers or 75 stars — or 90 / 90 / 225 when the repository owner submits it — and a repository at least 30 days old. Prefer a submission by a user |
| crates.io | `cargo install handrail` | From the first release |
| Install script | `curl -fsSL https://handrail.bitey.ai/install.sh \| sh` | Downloads a pinned release and verifies its checksum and signature. Interim domain; moves to `handrail.sh` once registered |

Constraints this puts on the design:

- **No self-updating binary.** Homebrew requires self-update to be disabled; upgrades go through
  the package manager that installed the binary. Rule-pack updates are a separate data channel
  (the signed index, §6), not a binary update.
- **An open-source licence compatible with the Debian Free Software Guidelines** (MIT and
  Apache-2.0 both qualify), stable tagged releases, and a build from source.

## 10. Internationalisation

English is the source of truth for code, docs, pack metadata and agent-facing rules.
Translations are optional overlays (`i18n/<locale>.toml`) for human-facing text only.
Agent-facing `rules.md` stays English by default; users can add local rules in any language.

## 11. Roadmap

| Milestone | Scope |
|---|---|
| **M0** (done) | Shell prototype: 8 security packs for Claude Code, 3 profiles, install/uninstall/status/dry-run, 56 tests |
| **M1** | Rust CLI; pack format v1; enforced + advisory tiers; Claude Code adapter; transactional apply; signed local registry |
| **M2** | Remote registry with signed releases; `update` with diff/approval; TUI; website catalog + configurator; contribution CI |
| **M3** | Non-security categories seeded (communication, git, quality, cost); user-defined profiles; import/export |
| **M4** | Second target adapter, chosen by demand, with a published enforcement matrix |

## 12. Risks

- **Supply chain** (see §6). Non-negotiable: signing, review, no auto-apply for executable content.
- **Fast-moving agent settings.** Keys appear and change frequently; adapters must be
  version-aware and CI must run against current agent releases.
- **False sense of security.** Mitigated by the enforcement matrix and per-pack `limits`.
- **Trademarks.** Do not use agent vendors' names or marks in the product name; state clearly
  that the project is unofficial.

## 13. Open questions

1. ~~Product name~~ — decided: Handrail. Repository under the maintainer's personal account (`martinx/handrail`); the `handrail` GitHub name is taken.
2. ~~License~~ — decided: **Apache-2.0**. Explicit patent grant for organisations deploying
   policy fleet-wide; contributions are inbound=outbound (Section 5), so community packs need no
   CLA; Section 6 withholds trademark rights, protecting the project name; accepted by Homebrew
   and crates.io.
3. Registry host: a public GitHub repo, with mirrors allowed.
4. Which second agent to support first — decided by user demand after v1.
