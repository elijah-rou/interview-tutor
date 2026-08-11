# Codex app-server compatibility

Interview Tutor uses only the official installed `codex app-server` stdio process and the CLI's ChatGPT login. It does not call OpenAI HTTP APIs, accept API keys, or read Codex authentication files.

## Tested protocol

Validated on 2025-08-11 with:

- executable: `/home/elijahrou/.bun/install/global/node_modules/@openai/codex/bin/codex.js` (canonical path reported by the test host)
- version: `codex-cli 0.146.0`
- stable generated schema: `codex_app_server_protocol.v2.schemas.json`, SHA-256 `37def830af431519597165a45a0d25840ff9cbe857d26556aa6b3d14db4cbf7a`
- generation: `codex app-server generate-json-schema --out <temporary-directory>` without `--experimental`
- login inspection: `codex login status` only; Interview Tutor never mutates login

The supported range is the shared stable subset in Codex CLI 0.146.x and 0.147.x: `initialize`, `initialized`, `account/read` with `refreshToken=false`, `thread/start`, `turn/start`, agent-message delta/item completion, turn completion/error, `turn/interrupt`, and account updates. Messages omit the `jsonrpc` field as required by these versions. Unknown notifications are ignored. Unknown or malformed responses to pending IDs fail closed.

Versions outside 0.146.x and 0.147.x are rejected with an actionable error. This is deliberate: approval and sandbox behavior must not be guessed across protocol changes.

## Security and privacy boundary

Each connection starts one child in a new process group and an empty temporary working directory. Its environment is cleared, then only authentication-location, executable search, locale, proxy, and certificate variables needed by the official client are restored. API-key variables are excluded. Threads are ephemeral, read-only, never-approve, and request disabled web search and no sandbox network access. Effective thread settings are checked before a turn.

These controls prevent local tool/file/network actions through the supported app-server contract. They are not a claim of total isolation from the hosted OpenAI service. Prompt content is sent to that service after visible consent and remains governed by the user's Codex account controls.

Transcripts are memory-only and bounded. They are cleared on reset, solve exit, and application exit. Interviewer and hinter use separate ephemeral threads, and the hinter receives no interviewer transcript.
