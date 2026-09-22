# Handrail

**Policy packs for coding agents — enforced where the agent allows it, honest where it doesn't.**

Handrail lets you browse, enable and disable rule packs for the coding agents you run
locally. It starts with Claude Code and security, and is designed to grow to more agents and
more categories. See [docs/design.md](docs/design.md).

> Unofficial project. Not affiliated with, or endorsed by, Anthropic or any agent vendor.
> Status: **prototype (M0)** — a POSIX-sh installer for Claude Code.

## Why

A line in an instructions file is a *request* to the model. A setting evaluated by the
agent's harness, a hook, or an OS sandbox is *enforcement*. Most "agent rules" today are the
first kind, and nothing tells you which is which. Handrail installs the second kind wherever
the agent supports it, and every module states what it protects, what it costs you, and what
it cannot do.

## Quick start (Claude Code, macOS or Linux)

```sh
git clone https://github.com/martinx/handrail && cd handrail
sh install.sh --list                          # profiles and modules
sh install.sh --profile baseline --dry-run    # show the plan; writes nothing
sudo sh install.sh --profile baseline         # install (asks for your password once)
sh install.sh --status                        # what is installed, and whether anything overrides it
sudo sh install.sh --uninstall --all          # remove everything
```

Then start a new `claude` session and run `/status`: the managed source should appear under
"Setting sources". `claude doctor` shows the same.

## Profiles

| Profile | Modules |
|---|---|
| `baseline` | `10-privacy` keep data on this machine · `20-anti-bypass` no bypassing · `60-audit` local audit log |
| `strict` | baseline + `30-secrets` credentials · `40-destructive` confirm destructive commands · `70-retention` 7-day local transcripts |
| `paranoid` | strict + `50-supply-chain` run only your own hooks · `80-sandbox` OS-level sandbox |

Each module's `module.json` lists what it protects, its tradeoffs, and its limits.

## How it works

Handrail writes to Claude Code's **managed policy directory**
(`/Library/Application Support/ClaudeCode/` on macOS, `/etc/claude-code/` on Linux):

- one file per module in `managed-settings.d/` — managed settings take precedence over user,
  project and command-line settings;
- a marked block in the managed `CLAUDE.md` — loaded in every session, cannot be excluded;
- hooks under `handrail/hooks/`.

The directory is owned by root, so an agent running as you cannot change or disable any of it.
Handrail never modifies files it did not create.

## Limits

- **Model inference sends your conversation to the model API.** No local setting changes
  that. Handrail turns off every *optional* upload and cloud execution path.
- A local administrator can edit the managed directory. Protecting against the administrator
  requires MDM.
- If an MDM or server-managed Claude Code policy exists, Claude Code ignores file-based
  policy by default. `install.sh --status` warns you when that is the case.

## Tests

```sh
sh tests/run.sh    # runs in a temporary directory; no root, touches nothing on the system
```

## License

To be decided (MIT or Apache-2.0).
