# Handrail

**Policy packs for coding agents — enforced where the agent allows it, honest where it doesn't.**

Handrail lets you browse, enable and disable rule packs for the coding agents you run
locally. It starts with Claude Code and security, and is designed to grow to more agents and
more categories. See [docs/design.md](docs/design.md) and [docs/plan.md](docs/plan.md).

> Unofficial project. Not affiliated with, or endorsed by, Anthropic or any agent vendor.
> Status: **0.1** — a Rust CLI for Claude Code on macOS and Linux.

## Why

A line in an instructions file is a *request* to the model. A setting evaluated by the
agent's harness, a hook, or an OS sandbox is *enforcement*. Most "agent rules" today are the
first kind, and nothing tells you which is which. Handrail installs the second kind wherever
the agent supports it, and every pack states what it protects, what it costs you, and what
it cannot do.

## Install

```sh
curl -fsSL https://handrail.bitey.ai/install.sh | sh   # recommended
brew install martinx/tap/handrail
cargo install handrail
```

The install script checks the release's SHA-256 and installs to `/usr/local/bin`, which only
root can write. That matters: Handrail runs itself through `sudo` to change enforced policy,
and a binary your user can overwrite (the Homebrew prefix, `~/.cargo/bin`) could be replaced
by anything running as you before `sudo` runs it. `handrail doctor` checks this.

Update with `handrail self-update` (Homebrew and cargo installs are pointed at their own
upgrade command).

## Use

```sh
handrail list                         # packs by category, with how strongly Claude Code enforces each
handrail show secrets                 # what a pack protects, its tradeoffs and its limits
handrail use baseline --dry-run       # the exact files a change would write; writes nothing
handrail use baseline                 # apply (asks for confirmation, then your password once)
handrail enable secrets               # add a pack
handrail disable audit                # remove a pack
handrail rule add "Reply in English"  # your own rule, added to the enforced instructions
handrail rollback                     # undo the last change
handrail status                       # what is installed, and anything that would make it ineffective
handrail doctor                       # checks, including tampering with installed files
handrail statusline                   # one line for Claude Code's status line
handrail self-update                  # update handrail itself
handrail disable --all                # remove everything; nothing of Handrail's is left behind
```

After a change, start a new `claude` session and run `/status`: the managed source appears
under "Setting sources". `claude doctor` shows the same.

## Profiles

| Profile | Packs |
|---|---|
| `baseline` | `privacy` keep data on this machine · `anti-bypass` no bypassing · `audit` local audit log |
| `strict` | baseline + `secrets` credentials · `destructive` confirm destructive commands · `retention` 7-day local transcripts |
| `paranoid` | strict + `supply-chain` run only your own hooks · `sandbox` OS-level sandbox |

## How it works

**Two tiers.** The agent runs as you, so anything you can change without administrator
rights, it can change too.

| | enforced | advisory |
|---|---|---|
| Where | Claude Code's managed policy directory (`/Library/Application Support/ClaudeCode/`, `/etc/claude-code/`): one `managed-settings.d/` fragment per pack, hooks, a marked block in `CLAUDE.md` | `~/.claude/rules/handrail-<pack>.md` |
| Owner | root — the agent cannot change or disable it | you |
| Changing it | asks for your password | no password |

**Changes are transactional.** Every change is planned first (`--dry-run` shows it), then
checked, validated, staged, backed up and journaled before any file is replaced. If the
process dies halfway, the next run restores the previous state exactly.

**The privileged step trusts nothing it is handed.** Only *which packs you want* is passed to
`sudo`. The privileged process recomputes the plan from the catalog compiled into the binary
and refuses unless it matches the plan you reviewed.

**Nothing it did not create is modified.** Other files in the managed directory are left
alone, and removing Handrail's block from `CLAUDE.md` restores the original bytes.

## Limits

- **Model inference sends your conversation to the model API.** No local setting changes
  that. Handrail turns off every *optional* upload and cloud execution path.
- A local administrator can edit the managed directory. Protecting against the administrator
  requires MDM.
- If an MDM or server-managed Claude Code policy exists, Claude Code ignores file-based
  policy by default. `handrail status` warns you when that is the case.

## Contributing

Releasing: [docs/RELEASING.md](docs/RELEASING.md). Packs live in [`catalog/`](catalog/): `pack.toml`, `rules.md`, per-agent settings and hooks,
and hook test vectors. `cargo test` validates every pack and runs its vectors.

## License

[Apache License 2.0](LICENSE). Contributions are accepted under the same license
(Section 5), so no separate contributor agreement is needed. The license does not grant
rights to the Handrail name (Section 6); see [NOTICE](NOTICE).
