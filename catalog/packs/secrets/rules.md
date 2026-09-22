### Credentials and secret files (secrets)

- Do not read, print or copy private keys, cloud credentials, `.env` files or token files. When a
  value is needed, ask the user to fill it in.
- If a password or key appears in the conversation, do not repeat it; remind the user it is now
  part of the session transcript.
- When generating credentials, write them only to a local file with mode 600 and tell the user
  where it is. Show them in the conversation only when the user explicitly asks.
