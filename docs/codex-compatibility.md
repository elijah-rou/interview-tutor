# Codex app-server compatibility

Interview Tutor defaults to the installed `codex app-server --stdio` executable and relies on the CLI's existing ChatGPT login. `INTERVIEW_TUTOR_CODEX_EXECUTABLE` may select an arbitrary executable for users and tests. The configured executable is a trusted user-selected boundary: it must be a regular file owned by the current user or root and must not be group- or world-writable. Version checks establish protocol compatibility, not provenance. They cannot authenticate same-user PATH entries, package-manager content, script interpreters, or other dependencies. Use the official `codex` executable from a trusted installation by default.

The wrapper does not call OpenAI HTTP APIs, accept API keys, inspect credentials, log prompts, or add tool-use instructions. The selected Codex process locates its own account state through `HOME` or `CODEX_HOME`.

## Tested protocol

Validated for this stack with these installed executables:

- `/home/elijahrou/.bun/install/global/node_modules/@openai/codex/bin/codex.js`, exactly `codex-cli 0.146.0`
- `/home/elijahrou/.npm/_npx/c8ab89660c602c20/node_modules/@openai/codex/bin/codex.js`, exactly `codex-cli 0.147.0`

Schemas were generated into separate temporary directories with `codex app-server generate-json-schema --out <temporary-directory>` without `--experimental`. Because 0.146.0 does not emit definitions in a stable object order, evidence uses canonical `jq -cS` output:

- 0.146.0 full `codex_app_server_protocol.v2.schemas.json`: SHA-256 `2f402b7d1356adccc1a4785c0656db457578ca9ea5d5b08953487a410c630ce8`
- 0.147.0 full `codex_app_server_protocol.v2.schemas.json`: SHA-256 `4422f141444d5531e549f4a3e8e7371c82e4dfbc6d5b6d06c8cd3dff8b4a8607`
- exact shared subset extraction: SHA-256 `d3187a04cbd0e7a46f3dda33934e1cfcb12415371fa2c0543663d216d71bafe5` for both versions, with byte-for-byte `cmp` success

The extracted definitions were `InitializeParams`, `GetAccountParams`, `GetAccountResponse`, `ThreadStartParams`, `ThreadStartResponse`, `TurnStartParams`, `TurnStartResponse`, `AgentMessageDeltaNotification`, `ItemCompletedNotification`, `TurnCompletedNotification`, `ErrorNotification`, `TurnInterruptParams`, and `TurnInterruptResponse`. The implementation additionally follows the generated stable server-request response schemas when it declines command execution, file changes, permission expansion, user input, and MCP elicitation. Unknown server requests fail closed.

Only exact versions 0.146.0 and 0.147.0 are accepted. The version probe has a 10-second wall bound, a combined 64-KiB stdout/stderr capture bound, cancellation polling, process-group and direct-PID TERM/KILL, nonblocking readers with bounded drains, reader joins, and bounded direct-child reap. The executable's device, inode, owner, mode, size, and change timestamps are rechecked immediately before app-server spawn. Other patch releases are not treated as verified merely because their minor version matches.

Messages omit the `jsonrpc` field as required by these versions. Responses must match a pending numeric request ID, with at most 16 pending IDs. Agent deltas and item completion are accepted only for the active thread, turn, and item. Turn completion and terminal errors must match the active thread and turn; retryable `willRetry` errors and unrelated events are ignored.

## Security and privacy boundary

The application payload has an explicit five-field allowlist: selected statement, current source, bounded latest test output, bounded in-memory transcript, and the user's question. Each connection uses a new empty mode-0700 temporary working directory. The child environment is cleared and restores only `HOME`, `CODEX_HOME`, `PATH`, locale, proxy, and certificate variables. `OPENAI_API_KEY` and unrelated environment variables are excluded. Threads request and verify their exact temporary cwd, ephemeral storage with a null path, read-only sandboxing, no sandbox network access, and never-approve policy; web search is requested disabled. The wrapper never accepts approval, permission, user-input, or MCP requests.

These controls limit what Interview Tutor intentionally sends, but they are not total process isolation. The configured Codex process may use read-only tools, configuration, or configured MCP servers and may access other locally readable paths permitted by its sandbox and configuration. In particular, `HOME` and `CODEX_HOME` remain available so the selected client can find account and configuration state. Use a dedicated Codex profile/home with minimal configuration for stronger isolation.

Prompt content is sent to OpenAI after visible consent and remains governed by the user's Codex account controls. Interview Tutor does not write prompt logs. Transcripts are memory-only and bounded, then cleared on reset, solve exit, and application exit. Interviewer and hinter use separate ephemeral threads, and the hinter receives no interviewer transcript. Explicit process shutdown removes temporary directories, including child-created files, and reports cleanup failures before a one-time session restart. Destructor cleanup is a documented best-effort fallback because destructors cannot return errors.

Safe manual smoke is limited to version, initialize, `initialized`, and `account/read` with `refreshToken=false`. It does not start a model turn.
