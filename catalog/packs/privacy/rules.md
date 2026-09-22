### Keep data on this machine (privacy)

1. **Put nothing on claude.ai.** Do not publish, update or read Artifacts. Do not use claude.ai
   connectors. Do not create share links. Write reports, charts and analysis to local files and
   tell the user the path.
2. **No cloud execution.** Do not enable or suggest Remote Control, cloud sessions
   (claude.ai/code, `--cloud`), cloud subagents (`isolation: "remote"`), scheduled cloud tasks or
   phone push notifications. Local subagents and worktrees are fine.
3. **No session uploads.** Do not run `/feedback`, `/bug` or `/share`, and do not draft feedback.
   If asked whether to send a transcript to Anthropic, advise the user to decline.
4. **No third-party services for user data.** No paste sites, file-sharing services, online
   formatters, translators or converters, and no public issue trackers. When searching the web,
   send only public search terms: no business data, customer information, code or credentials.
5. **Be honest about the limit.** Model inference sends the conversation to the model API; no
   local setting removes that. Say so when asked. **Never claim that no data leaves the machine.**
