#!/bin/sh
# ══════════════════════════════════════════════════════════════════════
# Handrail — policy packs for coding agents. This prototype targets Claude Code.
# Unofficial project; not affiliated with Anthropic.
#
#   sudo sh install.sh --profile baseline        install the baseline profile (recommended)
#   sudo sh install.sh --profile strict          install the strict profile
#   sudo sh install.sh --module 30-secrets       add a single module
#   sudo sh install.sh --profile strict --exact  make the installed set exactly this profile
#   sudo sh install.sh --add-rule "Reply in English"   add a rule of your own
#   sudo sh install.sh --uninstall --module 60-audit
#   sudo sh install.sh --uninstall --all         remove everything, leave nothing behind
#        sh install.sh --list | --status | --dry-run ...   (no sudo needed)
#
# ## Where it installs, and why
#
# Claude Code's managed policy directory (macOS: /Library/Application Support/ClaudeCode,
# Linux: /etc/claude-code). Settings there take top precedence: user settings, project
# settings and --settings cannot override them. The CLAUDE.md there is loaded in every
# session and cannot be excluded. The directory is owned by root, so an agent running as
# you cannot change it. That is what makes the policy stick.
#
# Each module becomes its own file in managed-settings.d/ (the documented fragment
# mechanism: files merge in name order, lists are unioned, nested blocks merge by key).
# Uninstalling a module deletes its file and touches nothing else in the directory.
#
# ## Dependencies: POSIX sh, sed, awk, grep
#
# This script runs as root. Fewer dependencies make it easier to review and less likely
# to surprise anyone on their machine.
# ══════════════════════════════════════════════════════════════════════
set -eu

VERSION="dev"
# Stamped by tools/release.sh. A dev build has no checksum and therefore refuses to
# bootstrap from the network.
TARBALL_SHA256=""
DEFAULT_BASE_URL="https://example.invalid/handrail"
MARK="handrail"

say()  { printf '%s\n' "$*"; }
warn() { printf 'WARNING: %s\n' "$*" >&2; }
die()  { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

# ── Bootstrap: when run as `curl ... | sh` there is no module directory next to us ──
#
# Download a pinned release, verify its SHA-256, and only then hand over to the
# install.sh inside it. The checksum is stamped into this script at release time, so the
# script and the tarball must come from the same release.
bootstrap() {
  [ -n "$TARBALL_SHA256" ] || die "This is a development build without a release checksum; refusing to download and install. Clone the repository and run: sh install.sh"
  base="${HANDRAIL_BASE_URL:-$DEFAULT_BASE_URL}"
  tmp=$(mktemp -d 2>/dev/null || mktemp -d -t handrail)
  trap 'rm -rf "$tmp"' EXIT INT TERM
  url="$base/$MARK-$VERSION.tar.gz"
  say "Downloading $url"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$tmp/pkg.tgz" || die "Download failed"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$url" -O "$tmp/pkg.tgz" || die "Download failed"
  else
    die "curl or wget is required"
  fi
  if command -v shasum >/dev/null 2>&1; then
    got=$(shasum -a 256 "$tmp/pkg.tgz" | cut -d' ' -f1)
  else
    got=$(sha256sum "$tmp/pkg.tgz" | cut -d' ' -f1)
  fi
  [ "$got" = "$TARBALL_SHA256" ] || die "Checksum mismatch: expected ${TARBALL_SHA256}, got ${got}. Nothing was changed."
  say "Checksum OK (SHA-256 ${got})"
  tar -xzf "$tmp/pkg.tgz" -C "$tmp"
  sh "$tmp/$MARK-$VERSION/install.sh" "$@"
  exit $?
}

case "$0" in
  *install.sh) SRC=$(cd "$(dirname "$0")" && pwd) ;;
  *) bootstrap "$@" ;;
esac
[ -d "$SRC/modules" ] || die "Module directory not found: $SRC/modules"

# ── Target directory ─────────────────────────────────────────────────
# HANDRAIL_ROOT is for tests only: it runs the whole flow in a temporary directory, without root.
case "$(uname -s)" in
  Darwin) OS=macos; SYS_ROOT="/Library/Application Support/ClaudeCode"; OWNER="root:wheel" ;;
  Linux)  OS=linux; SYS_ROOT="/etc/claude-code"; OWNER="root:root" ;;
  *)      die "$(uname -s) is not supported yet. On Windows, run inside WSL2." ;;
esac
ROOT="${HANDRAIL_ROOT:-$SYS_ROOT}"
TESTING=0; [ -n "${HANDRAIL_ROOT:-}" ] && TESTING=1
DDIR="$ROOT/managed-settings.d"
HOME_DIR="$ROOT/$MARK"
HOOKS="$HOME_DIR/hooks"
RULES="$HOME_DIR/rules"
STATE="$HOME_DIR/installed"
LOCAL_RULES="$HOME_DIR/local-rules.md"

# ── Arguments ────────────────────────────────────────────────────────
ACTION=install; DRY=0; YES=0; EXACT=0; ALL=0; PROFILE=""; RULE=""; MODS=""
while [ $# -gt 0 ]; do
  case "$1" in
    --profile)   [ $# -ge 2 ] || die "--profile needs a profile name"; PROFILE=$2; shift ;;
    --module)    [ $# -ge 2 ] || die "--module needs a module id"; MODS="$MODS $2"; shift ;;
    --uninstall) ACTION=uninstall ;;
    --all)       ALL=1 ;;
    --exact)     EXACT=1 ;;
    --add-rule)  [ $# -ge 2 ] || die "--add-rule needs the rule text"; ACTION=add-rule; RULE=$2; shift ;;
    --list)      ACTION=list ;;
    --status)    ACTION=status ;;
    --dry-run)   DRY=1 ;;
    --yes|-y)    YES=1 ;;
    -h|--help)   sed -n '2,13p' "$SRC/install.sh"; exit 0 ;;
    *)           die "Unknown argument: $1 (see --help)" ;;
  esac
  shift
done

# ── Helpers ──────────────────────────────────────────────────────────
mod_dir()  { printf '%s/modules/%s' "$SRC" "$1"; }
mod_field() { # mod_field <id> <key>: one string field from module.json
  grep -o "\"$2\"[[:space:]]*:[[:space:]]*\"[^\"]*\"" "$(mod_dir "$1")/module.json" | head -1 \
    | sed 's/^[^:]*:[[:space:]]*"//; s/"$//'
}
mod_list() { # mod_list <id> <key>: a string array from module.json, one item per line
  awk -v key="\"$2\"" '
    { buf = buf $0 "\n" }
    END {
      i = index(buf, key); if (!i) exit
      rest = substr(buf, i); s = index(rest, "["); e = index(rest, "]")
      arr = substr(rest, s + 1, e - s - 1)
      while (match(arr, /"[^"]*"/)) { print substr(arr, RSTART + 1, RLENGTH - 2); arr = substr(arr, RSTART + RLENGTH) }
    }' "$(mod_dir "$1")/module.json"
}
all_modules() { for d in "$SRC"/modules/*/; do basename "$d"; done; }
installed()   { [ -f "$STATE" ] && grep -v '^#' "$STATE" | grep -v '^$' || true; }
profile_mods() {
  f="$SRC/profiles/$1"; [ -f "$f" ] || die "No such profile: $1 (available: $(ls "$SRC/profiles" | tr '\n' ' '))"
  grep -v '^#' "$f" | grep -v '^[[:space:]]*$'
}
json_ok() { # 0 = valid, 1 = invalid, 2 = no tool available to check JSON
  if [ -x /usr/bin/plutil ]; then /usr/bin/plutil -convert xml1 -o /dev/null "$1" >/dev/null 2>&1; return $?; fi
  if command -v python3 >/dev/null 2>&1; then python3 -m json.tool "$1" >/dev/null 2>&1; return $?; fi
  if command -v jq >/dev/null 2>&1; then jq empty "$1" >/dev/null 2>&1; return $?; fi
  return 2
}
need_root() {
  [ "$TESTING" = 1 ] && return 0
  [ "$(id -u)" = 0 ] || die "This step writes a system directory and needs administrator rights: sudo sh $SRC/install.sh ${*:-}"
}
confirm() {
  [ "$YES" = 1 ] && return 0
  # When piped, stdin is the script itself, so the answer has to come from the terminal
  if [ -r /dev/tty ]; then
    printf '%s [y/N] ' "$1" > /dev/tty; read -r ans < /dev/tty || ans=""
    case "$ans" in y|Y|yes|YES) return 0 ;; esac
    die "Cancelled. Nothing was changed."
  fi
  die "No terminal to confirm on. Re-run with --yes once you have reviewed the plan."
}

# ── Other managed sources decide whether what we write is used at all ──
# Claude Code defaults to "first-wins": if an MDM or server-managed policy exists, the
# file-based policy is ignored entirely. Installed-but-ignored is the worst outcome,
# because the user believes they are protected. So say it.
other_sources() {
  if [ "$OS" = macos ] && [ -f "/Library/Managed Preferences/com.anthropic.claudecode.plist" ]; then
    warn "An MDM-delivered Claude Code policy (com.anthropic.claudecode) exists on this machine. By default it takes precedence and the file-based policy written by Handrail is IGNORED. Ask your IT administrator, or have them set managedSourcesBehavior: \"merge\" in the MDM policy."
  fi
  if [ -f "$ROOT/managed-settings.json" ]; then
    say "  Note: $ROOT/managed-settings.json exists (not written by Handrail). It is merged with Handrail's fragments; Handrail never modifies it."
  fi
}

# ── CLAUDE.md: maintain only our own block, keep everything else untouched ──
BEGIN_MARK="<!-- $MARK:begin (generated by Handrail; do not edit by hand, use install.sh) -->"
END_MARK="<!-- $MARK:end -->"
strip_block() { awk -v b="<!-- $MARK:begin" -v e="<!-- $MARK:end -->" '
  index($0, b) == 1 { skip = 1 } !skip { print } index($0, e) == 1 { skip = 0 }' "$1"; }
render_block() {
  say "$BEGIN_MARK"
  say "# Local policy (Handrail ${VERSION})"
  say ""
  say "> This block is owned by root, loaded in every session, and cannot be excluded by user"
  say "> settings. Hard limits are enforced by managed-settings.d/ and hooks in the same"
  say "> directory; this text is guidance for the agent. The two work together."
  for id in $(installed); do
    [ -f "$RULES/$id.md" ] && { say ""; cat "$RULES/$id.md"; }
  done
  if [ -s "$LOCAL_RULES" ]; then
    say ""; say "### Local rules"; say ""; cat "$LOCAL_RULES"
  fi
  say "$END_MARK"
}
write_claude_md() {
  f="$ROOT/CLAUDE.md"; tmp="$f.handrail.$$"
  # Never leave a half-written temporary file behind, even on failure
  trap 'rm -f "$tmp"' EXIT INT TERM
  if [ -f "$f" ]; then strip_block "$f" > "$tmp"; else : > "$tmp"; fi
  if [ -n "$(installed)" ] || [ -s "$LOCAL_RULES" ]; then
    [ -s "$tmp" ] && printf '\n' >> "$tmp"
    render_block >> "$tmp"
  fi
  # Only whitespace left means we created this file; remove it rather than leave it empty
  if grep -q '[^[:space:]]' "$tmp" 2>/dev/null; then mv "$tmp" "$f"; chmod 644 "$f"; else rm -f "$tmp" "$f"; fi
}

backup() {
  [ -d "$HOME_DIR" ] || return 0
  b="$HOME_DIR/backups/$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$b"
  cp -p "$DDIR/$MARK"-*.json "$b/" 2>/dev/null || true
  [ -f "$ROOT/CLAUDE.md" ] && cp -p "$ROOT/CLAUDE.md" "$b/" || true
  [ -f "$STATE" ] && cp -p "$STATE" "$b/" || true
  [ -f "$LOCAL_RULES" ] && cp -p "$LOCAL_RULES" "$b/" || true
  # Keep the five most recent
  ls -1d "$HOME_DIR"/backups/*/ 2>/dev/null | sort | head -n -5 2>/dev/null | while read -r old; do rm -rf "$old"; done || true
}

finish_perms() {
  [ "$TESTING" = 1 ] && return 0
  chown -R "$OWNER" "$ROOT"
  chmod 755 "$ROOT"
  [ -d "$DDIR" ] && chmod 755 "$DDIR"
  [ -d "$HOME_DIR" ] && chmod 755 "$HOME_DIR" "$HOOKS" "$RULES" 2>/dev/null || true
}

show_plan() { # show_plan <module...>
  for id in "$@"; do
    say ""
    say "  * $id  $(mod_field "$id" title)"
    say "    $(mod_field "$id" summary)"
    mod_list "$id" tradeoffs | while IFS= read -r t; do say "    Tradeoff: $t"; done
  done
}

# ══════════════════════════════════════════════════════════════════════
case "$ACTION" in

list)
  say "Profiles:"
  for p in "$SRC"/profiles/*; do
    say "  $(basename "$p"): $(sed -n '1s/^# *//p' "$p")"
    say "      $(grep -v '^#' "$p" | tr '\n' ' ')"
  done
  say ""; say "Modules:"
  for id in $(all_modules); do
    printf '  %-18s %-9s %s\n' "$id" "$(mod_field "$id" level)" "$(mod_field "$id" title)"
  done
  ;;

status)
  say "Handrail $VERSION · target $ROOT"
  if [ -z "$(installed)" ]; then say "  No modules installed."; else
    say "  Installed:"; for id in $(installed); do say "    + $id"; done
  fi
  [ -s "$LOCAL_RULES" ] && say "  Local rules: $(grep -c . "$LOCAL_RULES")"
  other_sources
  say ""
  say "  To confirm Claude Code picked it up: start a new session and run /status (see 'Setting sources'), or run: claude doctor"
  ;;

add-rule)
  need_root --add-rule "\"...\""
  [ "$DRY" = 1 ] && { say "[dry run] Would add rule: $RULE"; exit 0; }
  mkdir -p "$HOME_DIR"
  backup
  printf -- '- %s (added %s)\n' "$RULE" "$(date +%Y-%m-%d)" >> "$LOCAL_RULES"
  write_claude_md
  finish_perms
  say "Done. Takes effect in new sessions."
  ;;

install)
  want=""
  [ -n "$PROFILE" ] && want="$(profile_mods "$PROFILE")"
  want="$want $MODS"
  [ -n "$(printf '%s' "$want" | tr -d ' \n')" ] || die "Nothing to install. Use --profile baseline|strict|paranoid or --module <id> (see --list)"
  for id in $want; do [ -d "$(mod_dir "$id")" ] || die "No such module: $id"; done

  # Validate every JSON file before touching anything. A managed settings file that is
  # not valid JSON makes Claude Code refuse to start.
  for id in $want; do
    set +e; json_ok "$(mod_dir "$id")/settings.json"; rc=$?; set -e
    [ $rc = 1 ] && die "$id/settings.json is not valid JSON. Aborted; nothing was changed."
    [ $rc = 2 ] && warn "No plutil, python3 or jq on this machine to re-validate JSON (it was validated at release)."
  done

  if [ "$EXACT" = 1 ]; then target=$(printf '%s\n' $want | sort -u)
  else target=$( { installed; printf '%s\n' $want; } | grep -v '^$' | sort -u ); fi
  removing=$(installed | while read -r id; do printf '%s\n' "$target" | grep -qx "$id" || echo "$id"; done)
  adding=$(printf '%s\n' "$target" | while read -r id; do installed | grep -qx "$id" || echo "$id"; done)

  say "Handrail $VERSION -> $ROOT"
  other_sources
  if [ -n "$adding" ]; then say ""; say "Will install:"; show_plan $adding; fi
  if [ -n "$removing" ]; then say ""; say "Will remove (--exact): $(echo $removing)"; fi
  if [ -z "$adding" ] && [ -z "$removing" ]; then say ""; say "Everything requested is already installed; files will be refreshed from source."; fi
  [ "$DRY" = 1 ] && { say ""; say "[dry run] That was the plan. No files were written."; exit 0; }

  need_root "$@"
  say ""
  confirm "Continue?"
  mkdir -p "$DDIR" "$HOOKS" "$RULES"
  backup

  for id in $removing; do
    rm -f "$DDIR/$MARK-$id.json" "$RULES/$id.md" "$HOOKS/$id"-*.sh
  done
  for id in $target; do
    d=$(mod_dir "$id")
    if [ -d "$d/hooks" ]; then
      for h in "$d"/hooks/*.sh; do [ -f "$h" ] && cp "$h" "$HOOKS/" && chmod 755 "$HOOKS/$(basename "$h")"; done
    fi
    # Hook paths differ per platform, so they are filled in at install time. The macOS
    # path contains spaces; the settings files already quote it.
    sed "s|@HANDRAIL_HOOKS@|$HOOKS|g" "$d/settings.json" > "$DDIR/$MARK-$id.json"
    chmod 644 "$DDIR/$MARK-$id.json"
    cp "$d/rules.md" "$RULES/$id.md"
  done
  { echo "# Handrail $VERSION · $(date +%Y-%m-%dT%H:%M:%S)"; printf '%s\n' $target; } > "$STATE"
  write_claude_md
  finish_perms

  say ""
  say "Done. Installed: $(echo $(installed))"
  say "  Takes effect in new claude sessions (running sessions are not affected)."
  say "  To confirm: run /status in a session (see 'Setting sources'), or run: claude doctor"
  ;;

uninstall)
  if [ "$ALL" = 1 ]; then drop=$(installed); else drop="$MODS"; fi
  [ -n "$(printf '%s' "$drop" | tr -d ' \n')" ] || [ "$ALL" = 1 ] || die "Uninstall what? Use --module <id> or --all"
  say "Will uninstall: $(echo $drop)$( [ "$ALL" = 1 ] && echo ' (everything, including local rules and backups)')"
  [ "$DRY" = 1 ] && { say "[dry run] No files were removed."; exit 0; }
  need_root --uninstall
  confirm "Continue?"

  if [ "$ALL" = 1 ]; then
    # Full removal: snapshot to the temp directory (cleared on reboot) so nothing of ours
    # remains in the system directory
    if [ -d "$HOME_DIR" ]; then
      snap="${TMPDIR:-/tmp}/$MARK-uninstalled-$(date +%Y%m%d-%H%M%S).tar.gz"
      tar -czf "$snap" -C "$ROOT" "$MARK" $(cd "$ROOT" && ls managed-settings.d/"$MARK"-*.json 2>/dev/null) 2>/dev/null || true
      say "  Snapshot before removal: $snap (temporary directory, cleared on reboot)"
    fi
    rm -f "$DDIR/$MARK"-*.json
    rm -rf "$HOME_DIR"
    [ -d "$DDIR" ] && rmdir "$DDIR" 2>/dev/null || true
    LOCAL_RULES="/nonexistent"; STATE="/nonexistent"
    [ -f "$ROOT/CLAUDE.md" ] && write_claude_md
    rmdir "$ROOT" 2>/dev/null || true
    say "Done. Everything was removed."
  else
    backup
    for id in $drop; do
      installed | grep -qx "$id" || { warn "$id was not installed; skipping"; continue; }
      rm -f "$DDIR/$MARK-$id.json" "$RULES/$id.md" "$HOOKS/$id"-*.sh
      tmp="$STATE.$$"; grep -vx "$id" "$STATE" > "$tmp" || true; mv "$tmp" "$STATE"
    done
    write_claude_md
    finish_perms
    say "Done. Removed: $(echo $drop). Remaining: $(echo $(installed))"
  fi
  ;;
esac
