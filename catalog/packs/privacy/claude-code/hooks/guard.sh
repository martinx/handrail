#!/bin/sh
# Handrail · privacy: block tool calls that would move data off this machine
# onto claude.ai, before they run.
#
# Settings already deny most of these. This hook covers what settings cannot:
#   - cloud subagents (isolation: "remote") have no per-device setting
#   - future cloud tools whose names match are blocked before a policy update ships
# Exit code 2 blocks the call; stderr is returned to the model so it knows why.
# Only /bin/sh, grep and sed: a broken managed hook blocks every session,
# so fewer dependencies means fewer ways to fail.

input=$(cat)
tool=$(printf '%s' "$input" | grep -o '"tool_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed 's/.*"\([^"]*\)"$/\1/')

block() {
  echo "Handrail blocked this: ${1} (local policy: data stays on this machine)" >&2
  exit 2
}

case "$tool" in
  Artifact|ArtifactComments|ArtifactData) block "publishing or reading Artifacts on claude.ai" ;;
  RemoteTrigger)                          block "creating a scheduled cloud task" ;;
  PushNotification)                       block "sending a push notification through the cloud" ;;
  SendFeedback)                           block "drafting or sending feedback (it attaches the transcript)" ;;
  mcp__claude_ai_*)                       block "using the claude.ai connector ${tool}" ;;
  Agent|Task|Workflow)
    # Local subagents are fine; only block the ones that run in a cloud environment
    if printf '%s' "$input" | grep -q '"isolation"[[:space:]]*:[[:space:]]*"remote"'; then
      block "starting an agent in a cloud environment (isolation: remote)"
    fi
    ;;
esac
exit 0
