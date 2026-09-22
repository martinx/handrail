# Releasing

One tag publishes everywhere. Nothing is uploaded by hand.

```sh
# 1. bump the version in Cargo.toml, commit, push to main, wait for CI
# 2. tag it
git tag -a v0.1.2 -m "Handrail 0.1.2"
git push origin v0.1.2
```

## What the tag publishes

| Channel | How users get it | Done by |
|---|---|---|
| GitHub Release — binaries for `aarch64/x86_64-apple-darwin`, `x86_64/aarch64-unknown-linux-musl`, `SHA256SUMS`, build-provenance attestations | `curl -fsSL https://handrail.bitey.ai/install.sh \| sh`, `handrail self-update` | `release.yml` → `build`, `github-release` |
| crates.io | `cargo install handrail` | `release.yml` → `crates-io` (trusted publishing) |
| Homebrew tap `martinx/homebrew-tap` | `brew install martinx/tap/handrail` | `release.yml` → `homebrew` (formula rendered from `SHA256SUMS`) |
| Website `handrail.bitey.ai` (version shown, pack catalog) | — | `pages.yml`, triggered by the same tag |

`release.yml` refuses a tag that differs from the `version` in `Cargo.toml`.

## One-time setup (already done unless noted)

| What | Where | Notes |
|---|---|---|
| GitHub Pages: source "GitHub Actions", custom domain `handrail.bitey.ai`, HTTPS enforced | repository settings | DNS: `handrail CNAME martinx.github.io.` in DNSPod |
| Verified domain `bitey.ai` for GitHub Pages | account settings → Pages | Prevents subdomain takeover. TXT `_github-pages-challenge-martinx` in DNSPod |
| `HOMEBREW_TAP_DEPLOY_KEY` secret | `martinx/handrail` → Actions secrets | Private half of a deploy key with write access to `martinx/homebrew-tap` only. The key exists nowhere else. To rotate: generate a new key, replace the deploy key on the tap and this secret |
| crates.io trusted publisher | crates.io → `handrail` → Settings → Trusted Publishing | GitHub, owner `martinx`, repository `handrail`, workflow `release.yml`, no environment. No token is stored anywhere |

## Not published (yet)

- **homebrew-core** (`brew install handrail` without the tap): needs 75 stars / 30 forks / 30 watchers
  when submitted by a user (225 / 90 / 90 when submitted by the owner) and a repository at least
  30 days old. Prefer a user submission.
- Windows, Nix, AUR, apt: not before there is demand.

## If a channel fails

Re-run only the failed job: `gh run rerun <run-id> --failed`. The jobs are independent after
`github-release`; a failed crates.io or Homebrew job does not affect the GitHub Release.
