#!/bin/sh
# Handrail · audit: append one JSON line per tool call. Local only; never sent anywhere.
#
# Location: ~/.claude/handrail/audit/YYYY-MM-DD.jsonl (directory 700, files 600).
# Always exits 0: a failing audit must never block real work.

input=$(cat)
dir="${HOME}/.claude/handrail/audit"
umask 077
mkdir -p "$dir" 2>/dev/null || exit 0

field() { printf '%s' "$input" | grep -o "\"$1\"[[:space:]]*:[[:space:]]*\"[^\"]*\"" | head -1 | sed 's/.*:[[:space:]]*"\(.*\)"$/\1/'; }
tool=$(field tool_name)
session=$(field session_id)
cwd=$(field cwd)
# Keep the first 300 characters of a command and mask common password patterns.
# Best effort, not a guarantee.
cmd=$(field command | cut -c1-300 | sed -E 's/(password|passwd|token|secret|api[_-]?key)=[^ ]*/\1=***/Ig; s/(PGPASSWORD|MYSQL_PWD)=[^ ]*/\1=***/g')
file=$(field file_path)

ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
printf '{"ts":"%s","session":"%s","cwd":"%s","tool":"%s","command":"%s","file":"%s"}\n' \
  "$ts" "$session" "$cwd" "$tool" "$cmd" "$file" >> "$dir/$(date -u +%Y-%m-%d).jsonl" 2>/dev/null
exit 0
