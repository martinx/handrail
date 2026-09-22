#!/bin/sh
# End-to-end tests. The whole install flow runs in a temporary directory (HANDRAIL_ROOT):
# no root required, nothing on the system is touched.
set -u
cd "$(dirname "$0")/.."
pass=0; fail=0
ok()   { pass=$((pass+1)); printf '  ok   %s\n' "$1"; }
bad()  { fail=$((fail+1)); printf '  FAIL %s\n' "$1"; }
check(){ if eval "$2"; then ok "$1"; else bad "$1"; fi; }

echo "-- 1. Module structure and JSON"
for d in modules/*/; do
  id=$(basename "$d")
  for f in module.json settings.json rules.md; do [ -s "$d$f" ] || bad "$id is missing $f"; done
  python3 -m json.tool "$d/module.json" >/dev/null 2>&1 && python3 -m json.tool "$d/settings.json" >/dev/null 2>&1 \
    && ok "$id: JSON is valid" || bad "$id: invalid JSON"
  grep -q "\"id\": \"$id\"" "$d/module.json" && ok "$id: id matches directory name" || bad "$id: id does not match directory name"
done

echo "-- 1b. Static check: \$VAR must not be followed directly by a non-ASCII character"
# sh/bash swallow the leading bytes of a multibyte character into the variable name:
# "\$VERSION）" is read as a variable named VERSION\xef\xbc, which aborts under set -u.
# This bit the prototype three times; once it stopped an install halfway. Use \${VAR}.
hits=$(perl -ne 'print "    $ARGV:$.: $_" if /\$[A-Za-z_][A-Za-z0-9_]*[^\x00-\x7F]/; close ARGV if eof' install.sh modules/*/hooks/*.sh tools/*.sh 2>/dev/null)
[ -z "$hits" ] && ok "all scripts use the \${VAR} form" || { bad "found \$VAR followed by non-ASCII:"; printf '%s\n' "$hits"; }

echo "-- 1c. Shipped text is English (the project defaults to English)"
hits=$(perl -ne 'print "    $ARGV:$.: $_" if /[\x{4e00}-\x{9fff}]/; close ARGV if eof' -CSD install.sh profiles/* modules/*/*.json modules/*/*.md modules/*/hooks/* README.md docs/*.md 2>/dev/null)
[ -z "$hits" ] && ok "no CJK characters in shipped files" || { bad "CJK characters found:"; printf '%s\n' "$hits" | head -5; }

echo "-- 2. Modules must not set the same scalar key to different values (the later file would silently win)"
python3 - <<'PY' && ok "no scalar conflicts" || bad "scalar conflicts found"
import json, glob, sys
seen = {}; bad = False
def walk(o, p, src):
    global bad
    if isinstance(o, dict):
        for k, v in o.items(): walk(v, p + [k], src)
    elif isinstance(o, list): return
    else:
        key = ".".join(p)
        if key in seen and seen[key][0] != o:
            print(f"    conflict {key}: {seen[key][1]}={seen[key][0]} vs {src}={o}"); bad = True
        seen[key] = (o, src)
for f in sorted(glob.glob("modules/*/settings.json")):
    walk(json.load(open(f)), [], f.split("/")[1])
sys.exit(1 if bad else 0)
PY

echo "-- 3. Hook test vectors (every .cases file)"
for c in modules/*/hooks/*.cases; do
  h="${c%.cases}.sh"
  while IFS='	' read -r name want input; do
    case "$name" in \#*|'') continue ;; esac
    got=$(printf '%s' "$input" | HOME="$(mktemp -d)" sh "$h" >/dev/null 2>&1; echo $?)
    [ "$got" = "$want" ] && ok "$(basename "$h"): $name" || bad "$(basename "$h"): $name (expected ${want}, got ${got})"
  done < "$c"
done

echo "-- 4. The audit hook masks passwords"
H=$(mktemp -d)
printf '%s' '{"session_id":"s","cwd":"/x","tool_name":"Bash","tool_input":{"command":"mysql --password=hunter2 -e 1"}}' | HOME="$H" sh modules/60-audit/hooks/60-audit-log.sh
check "password is masked" "! grep -rq hunter2 '$H/.claude/handrail/audit' && grep -rq 'password=\*\*\*' '$H/.claude/handrail/audit'"
check "log file mode is 600" "[ \"\$(stat -f %Lp \"\$(ls '$H'/.claude/handrail/audit/*.jsonl)\" 2>/dev/null || stat -c %a \"\$(ls '$H'/.claude/handrail/audit/*.jsonl)\")\" = 600 ]"

echo "-- 5. Install flow (temporary directory)"
R=$(mktemp -d)/ClaudeCode
export HANDRAIL_ROOT="$R"
mkdir -p "$R/managed-settings.d"
printf '# Rules from your organisation\nCompany rule A\n' > "$R/CLAUDE.md"
echo '{"model":"opus"}' > "$R/managed-settings.d/00-org.json"

sh install.sh --profile baseline --dry-run >/dev/null 2>&1
check "dry run writes nothing" "[ ! -e '$R/handrail' ] && ! ls '$R'/managed-settings.d/handrail-* >/dev/null 2>&1"

sh install.sh --profile baseline --yes >/dev/null 2>&1
check "baseline installs 3 modules" "[ \$(ls '$R'/managed-settings.d/handrail-*.json | wc -l) -eq 3 ]"
check "hook path placeholder is substituted" "! grep -q @HANDRAIL_HOOKS@ '$R'/managed-settings.d/handrail-*.json && grep -q \"$R/handrail/hooks/10-privacy-guard.sh\" '$R/managed-settings.d/handrail-10-privacy.json'"
check "still valid JSON after substitution" "for f in '$R'/managed-settings.d/handrail-*.json; do python3 -m json.tool \"\$f\" >/dev/null || exit 1; done"
check "hooks are executable" "[ -x '$R/handrail/hooks/10-privacy-guard.sh' ] && [ -x '$R/handrail/hooks/60-audit-log.sh' ]"
check "existing CLAUDE.md content is preserved" "grep -q 'Company rule A' '$R/CLAUDE.md'"
check "CLAUDE.md contains our block" "grep -q 'handrail:begin' '$R/CLAUDE.md' && grep -q 'Keep data on this machine' '$R/CLAUDE.md'"
check "an existing fragment file is untouched" "[ \"\$(cat '$R/managed-settings.d/00-org.json')\" = '{\"model\":\"opus\"}' ]"
check "no temporary files left behind" "! ls '$R'/*.handrail.* >/dev/null 2>&1"

sh install.sh --profile baseline --yes >/dev/null 2>&1
check "reinstall is idempotent (one block in CLAUDE.md)" "[ \$(grep -c 'handrail:begin' '$R/CLAUDE.md') -eq 1 ]"

sh install.sh --profile strict --yes >/dev/null 2>&1
check "upgrading to strict gives 6 modules" "[ \$(ls '$R'/managed-settings.d/handrail-*.json | wc -l) -eq 6 ]"

sh install.sh --profile baseline --exact --yes >/dev/null 2>&1
check "--exact back to baseline leaves 3" "[ \$(ls '$R'/managed-settings.d/handrail-*.json | wc -l) -eq 3 ] && ! grep -q 30-secrets '$R/CLAUDE.md'"

sh install.sh --add-rule "Reply in English" --yes >/dev/null 2>&1
check "local rule lands in CLAUDE.md" "grep -q 'Reply in English' '$R/CLAUDE.md'"

sh install.sh --uninstall --module 60-audit --yes >/dev/null 2>&1
check "uninstall a single module" "[ ! -e '$R/managed-settings.d/handrail-60-audit.json' ] && [ ! -e '$R/handrail/hooks/60-audit-log.sh' ] && ! grep -q '60-audit' '$R/handrail/installed'"

sh install.sh --uninstall --all --yes >/dev/null 2>&1
check "full uninstall leaves none of our files" "[ ! -e '$R/handrail' ] && ! ls '$R'/managed-settings.d/handrail-* >/dev/null 2>&1"
check "full uninstall keeps the organisation's files" "grep -q 'Company rule A' '$R/CLAUDE.md' && ! grep -q 'handrail' '$R/CLAUDE.md' && [ -f '$R/managed-settings.d/00-org.json' ]"

R2=$(mktemp -d)/ClaudeCode; export HANDRAIL_ROOT="$R2"
sh install.sh --profile baseline --yes >/dev/null 2>&1; sh install.sh --uninstall --all --yes >/dev/null 2>&1
check "install from scratch then remove all: directory disappears" "[ ! -e '$R2' ]"

echo "-- 6. Bad input"
check "unknown profile is an error" "! sh install.sh --profile nope --yes >/dev/null 2>&1"
check "unknown module is an error" "! sh install.sh --module 99-nope --yes >/dev/null 2>&1"
unset HANDRAIL_ROOT
check "installing to the system directory without sudo is refused" "! sh install.sh --profile baseline --yes >/dev/null 2>&1 && [ ! -d '/Library/Application Support/ClaudeCode/handrail' ]"
check "a dev build refuses to bootstrap from the network" "cat install.sh | sh -s -- --profile baseline 2>&1 | grep -q 'development build'"

echo; echo "passed $pass · failed $fail"; [ "$fail" = 0 ]
