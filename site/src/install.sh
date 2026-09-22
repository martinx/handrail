#!/bin/sh
# Handrail installer
#
#   curl -fsSL https://handrail.bitey.ai/install.sh | sh
#
# Downloads the handrail binary for this OS and architecture from GitHub Releases, checks
# its SHA-256 against the release's SHA256SUMS, and installs it to /usr/local/bin. Only the
# final copy uses sudo; downloading and checking run as you. It does not change any agent
# configuration — run `handrail use baseline` yourself afterwards.
#
# Options (environment variables):
#   HANDRAIL_VERSION=v0.1.0          install a specific release instead of the latest
#   HANDRAIL_INSTALL_DIR=/some/dir   install somewhere else (see the note on writable dirs below)
#   HANDRAIL_VERIFY_PROVENANCE=1     also require `gh attestation verify` to pass
#
# The whole script is one function called on the last line, so a download cut off halfway
# runs nothing.

set -eu

REPO="martinx/handrail"

say()  { printf '%s\n' "$*"; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

download() { # download <url> <file>
  if have curl; then
    curl -fsSL --proto '=https,file' --tlsv1.2 "$1" -o "$2" || die "download failed: $1"
  elif have wget; then
    wget -q --https-only "$1" -O "$2" || die "download failed: $1"
  else
    die "curl or wget is required"
  fi
}

sha256() {
  if have shasum; then shasum -a 256 "$1" | cut -d' ' -f1
  elif have sha256sum; then sha256sum "$1" | cut -d' ' -f1
  else die "shasum or sha256sum is required to verify the download"
  fi
}

main() {
  have tar || die "tar is required"

  case "$(uname -s)/$(uname -m)" in
    Darwin/arm64)                target=aarch64-apple-darwin ;;
    Darwin/x86_64)               target=x86_64-apple-darwin ;;
    Linux/x86_64|Linux/amd64)    target=x86_64-unknown-linux-musl ;;
    Linux/aarch64|Linux/arm64)   target=aarch64-unknown-linux-musl ;;
    *) die "$(uname -s) $(uname -m) is not supported yet (macOS and Linux on x86_64 or arm64). On Windows, use WSL2." ;;
  esac

  version="${HANDRAIL_VERSION:-latest}"
  if [ "$version" = latest ]; then
    base="https://github.com/$REPO/releases/latest/download"
  else
    base="https://github.com/$REPO/releases/download/$version"
  fi
  base="${HANDRAIL_DOWNLOAD_BASE:-$base}" # for testing against a local directory
  asset="handrail-${target}.tar.gz"

  tmp=$(mktemp -d 2>/dev/null || mktemp -d -t handrail)
  trap 'rm -rf "$tmp"' EXIT INT TERM

  say "Downloading Handrail ($version, $target)"
  download "$base/$asset" "$tmp/$asset"
  download "$base/SHA256SUMS" "$tmp/SHA256SUMS"

  expected=$(awk -v f="$asset" '$2 == f || $2 == "*"f { print $1 }' "$tmp/SHA256SUMS")
  [ -n "$expected" ] || die "SHA256SUMS has no entry for $asset"
  actual=$(sha256 "$tmp/$asset")
  [ "$expected" = "$actual" ] || die "checksum mismatch for $asset (expected ${expected}, got ${actual}); nothing was installed"
  say "Checksum OK"

  if [ "${HANDRAIL_VERIFY_PROVENANCE:-0}" = 1 ]; then
    have gh || die "HANDRAIL_VERIFY_PROVENANCE=1 needs the GitHub CLI (gh)"
    gh attestation verify "$tmp/$asset" --repo "$REPO" >/dev/null || die "provenance verification failed; nothing was installed"
    say "Provenance OK (built by $REPO's release workflow)"
  fi

  tar -xzf "$tmp/$asset" -C "$tmp"
  [ -f "$tmp/handrail" ] || die "the archive does not contain a handrail binary"
  "$tmp/handrail" --version >/dev/null 2>&1 || die "the downloaded binary does not run on this machine"

  dir="${HANDRAIL_INSTALL_DIR:-/usr/local/bin}"
  # Handrail runs itself through sudo to change enforced policy. It must live where only
  # root can write; otherwise anything running as you could replace it before sudo runs it.
  if [ -d "$dir" ] && [ -w "$dir" ]; then
    install -m 755 "$tmp/handrail" "$dir/handrail"
    case "$dir" in /usr/local/bin|/usr/bin|/opt/*) ;; *)
      say "Note: $dir is writable by you. For enforced policy, prefer a directory only root can write (see: handrail doctor)." ;;
    esac
  else
    say "Installing to $dir (needs your password once)"
    have sudo || die "sudo is required to write $dir; or set HANDRAIL_INSTALL_DIR"
    sudo mkdir -p "$dir"
    sudo install -m 755 "$tmp/handrail" "$dir/handrail"
  fi

  say ""
  say "Installed $("$dir/handrail" --version) to $dir/handrail"
  say ""
  say "Next:"
  say "  handrail list                     # packs, and how strongly Claude Code enforces each"
  say "  handrail use baseline --dry-run   # see exactly what would change"
  say "  handrail use baseline             # apply"
  case ":$PATH:" in *":$dir:"*) ;; *) say ""; say "$dir is not on your PATH; add it, or run $dir/handrail" ;; esac
}

main "$@"
