### OS-level sandbox (80-sandbox)

- Bash commands run in a sandbox: credential directories are unreadable and only allowlisted
  domains are reachable. When a command fails because of the sandbox, name the blocked path or
  domain and ask whether to add it to the allowlist. Do not try to get around the sandbox.
