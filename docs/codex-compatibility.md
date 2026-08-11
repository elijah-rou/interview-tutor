# Codex setup, compatibility, and privacy

Codex Interview is optional. Local browsing, editing, testing, and submission work without it.

## Setup

1. Install the Codex CLI from a trusted source.
2. Authenticate through the CLI:

   ```console
   codex login
   codex login status
   ```

3. Run `./interview`, focus Interview with `i`, read the disclosure, and accept with Enter/`y`.

Interview Tutor defaults to the `codex` found on `PATH` and starts `codex app-server --stdio`. `INTERVIEW_TUTOR_CODEX_EXECUTABLE=/absolute/path/to/codex` may select a different executable. That configured executable is a trusted user-selected boundary. It must resolve to a regular file owned by the current user or root and not be group- or world-writable. Its device, inode, owner, mode, size, and change timestamps are checked again immediately before app-server spawn.

Those checks and the version probe establish compatibility, not provenance. They do not authenticate same-user PATH entries, package-manager content, scripts/interpreters, or dependencies. Prefer an official Codex executable from a trusted installation. Use `./interview --no-codex` to prevent even a version probe or process spawn.

Interview Tutor does not read OpenAI API keys, login tokens, or Codex credential files. It does not call OpenAI HTTP APIs directly. The selected Codex process reads its own account/configuration state through `HOME` or `CODEX_HOME`. Do not pass or paste credentials into the Interview composer.

## Exact supported protocol

Only exact Codex CLI versions 0.146.0 and 0.147.0 are accepted. Another patch or minor version is not treated as compatible merely because its version string is close.

This stack validated these installed executables:

- `/home/elijahrou/.bun/install/global/node_modules/@openai/codex/bin/codex.js`, exactly `codex-cli 0.146.0`
- `/home/elijahrou/.npm/_npx/c8ab89660c602c20/node_modules/@openai/codex/bin/codex.js`, exactly `codex-cli 0.147.0`

Schemas were generated into separate temporary directories with `codex app-server generate-json-schema --out <temporary-directory>` without `--experimental`. Since 0.146.0 does not emit definitions in stable object order, evidence uses canonical `jq -cS` output:

- 0.146.0 full `codex_app_server_protocol.v2.schemas.json`: SHA-256 `2f402b7d1356adccc1a4785c0656db457578ca9ea5d5b08953487a410c630ce8`
- 0.147.0 full schema: SHA-256 `4422f141444d5531e549f4a3e8e7371c82e4dfbc6d5b6d06c8cd3dff8b4a8607`
- exact shared subset: SHA-256 `d3187a04cbd0e7a46f3dda33934e1cfcb12415371fa2c0543663d216d71bafe5` for both versions, with byte-for-byte `cmp` success

The extracted definitions were `InitializeParams`, `GetAccountParams`, `GetAccountResponse`, `ThreadStartParams`, `ThreadStartResponse`, `TurnStartParams`, `TurnStartResponse`, `AgentMessageDeltaNotification`, `ItemCompletedNotification`, `TurnCompletedNotification`, `ErrorNotification`, `TurnInterruptParams`, and `TurnInterruptResponse`. The client also follows the generated server-request response schemas when declining command execution, file changes, permission expansion, user input, and MCP elicitation. Unknown server requests fail closed.

Messages omit `jsonrpc` as required by these versions. Responses must match a pending numeric request ID, with at most 16 pending IDs. Deltas and item completion must match the active thread/turn/item; terminal errors and completion must match the active thread and turn. Retryable `willRetry` errors and unrelated events are ignored.

The version probe has a 10-second wall bound and combined 64-KiB stdout/stderr capture. Startup requests have 10-second bounds, turns 120 seconds, interrupt acknowledgement 2 seconds, and shutdown 2 seconds followed by bounded kill/reap and reader drains. Protocol lines are limited to 2 MiB and assistant content to 64 KiB.

## Disclosure and outbound data

After visible consent, Interview Tutor intentionally supplies exactly five application fields to the configured process:

1. selected local statement
2. current source revision
3. bounded latest local test output
4. bounded in-memory transcript
5. current user question

The latest output included in a turn is capped to its most recent 16 KiB. Questions/composer content are capped at 16 KiB. The transcript retains at most 128 entries and 256 KiB; each assistant response is at most 64 KiB. Prompt content is then sent to OpenAI by the configured Codex client and remains governed by the user's Codex account controls.

Each connection uses a new empty mode-0700 temporary cwd. The child environment is cleared and restores only `HOME`, `CODEX_HOME`, `PATH`, locale, proxy, and certificate variables. `OPENAI_API_KEY` and unrelated environment variables are excluded. Threads request and verify the exact temporary cwd, ephemeral storage with a null path, read-only sandboxing, no sandbox network access, never-approve policy, and disabled web search. Interview Tutor refuses command, file-change, permission, user-input, and MCP approval requests.

These controls limit the application's intended payload, but they are not total process isolation. The configured Codex executable may use its read-only tools, configuration, or MCP servers and may access other readable local paths allowed by its sandbox/configuration. `HOME` and `CODEX_HOME` remain available for account/configuration lookup. A compromised or unexpectedly configured executable is outside Interview Tutor's isolation boundary. Use a dedicated Codex profile/home with minimal readable data, tools, and MCP configuration for stronger separation.

## Session behavior

Interviewer and hinter use separate ephemeral app-server threads. The interviewer asks focused Socratic questions and receives the bounded transcript. Hints omit the transcript and are limited to three levels per source revision; no hint may return complete language code. Editing starts a new hint allowance.

Submission review begins only after local attempt recording succeeds. It receives the exact source captured by that submit operation, not a newer editor buffer, and reviews correctness, complexity, edge cases, and communication. The local runner and recorded outcome remain authoritative.

A response enters the UI and transcript only while operation, role, and source revision still match. Stale responses are discarded. Transcript state is memory-only and cleared on `Space r`, solve exit, and application exit; Interview Tutor writes no prompt/transcript log. The temporary submitted-source copy is wiped when dropped. Explicit process shutdown removes the temporary cwd and reports cleanup failures; destructor cleanup is best effort only.

## Troubleshooting

- **`auth` / authentication required:** run `codex login`, confirm with `codex login status`, then press `i` to reconnect for the next operation.
- **unsupported version:** install exact 0.146.0 or 0.147.0 and confirm the selected binary with `codex --version`. If using an override, verify `INTERVIEW_TUTOR_CODEX_EXECUTABLE` points to the intended trusted file.
- **executable rejected:** correct its owner/permissions or select a trusted regular file. Group- or world-writable executables are rejected.
- **protocol error, malformed response, EOF, timeout, or missing interrupt acknowledgement:** the process is invalidated. Press `i` to make one fresh connection for the next distinct operation. Failed request content is not replayed, and a session permits at most one replacement process.
- **offline/client failure:** local edit, F5/Ctrl-S tests, and F9 submission still work. Restart with `--no-codex` to disable the integration completely.
- **privacy boundary is too broad:** decline disclosure or use `--no-codex`. Otherwise use a dedicated minimal `CODEX_HOME`/profile and remove unneeded read-only tools and MCP configuration.

A safe manual compatibility smoke is limited to version, initialize, `initialized`, and `account/read` with `refreshToken=false`. It does not start a model turn. Automated tests use the checked-in fake app-server only and never invoke a live model.
