# Interviewer setup, compatibility, and privacy

The interviewer is optional. Local browsing, Neovim editing, testing, submission, and navigation work without it and continue working after interviewer failures.

## Backend selection

`./interview` resolves exactly one backend in this order:

1. `--interviewer pi|codex|none`
2. legacy `--no-codex`, mapped to `none`
3. `INTERVIEW_TUTOR_INTERVIEWER`
4. default `pi`

Using both CLI selectors is an error. Empty, non-UTF-8, mixed-case, whitespace-padded, and unknown selected values are rejected. There is no silent fallback between Pi and Codex. `none` probes or spawns neither executable.

Both transports require visible in-app consent before version probing, authentication/configuration access, or model turns. They receive the same application-owned prompt, transcript, hint, structured-envelope, revision, and stale-response validation.

## Pi default

Install trusted Pi 0.84.2, configure its default model/provider and credentials, then run `./interview`. The executable resolves from `INTERVIEW_TUTOR_PI_EXECUTABLE` and then `PATH`. Only exact `pi --version` output `0.84.2` is accepted.

The resolved executable must be a regular file owned by the effective user or root and must not be group- or world-writable. Device, inode, owner, mode, size, and change timestamps are checked before and after the bounded version probe and again before RPC spawn. This reduces replacement races and establishes compatibility, not provenance.

Every application turn starts a fresh process group in a new empty mode-0700 cwd with this exact argv:

```console
pi --mode rpc --no-session --no-tools --no-extensions --no-skills \
  --no-prompt-templates --no-themes --no-context-files --no-approve --offline
```

The process is retained only for one structured-output correction, then terminated and reaped. The environment is cleared. Interview Tutor restores a bounded allowlist for `HOME`, `PATH`, locale, proxy, certificate, Pi agent/package directories, documented provider credential variables, and cloud-provider authentication variables. It forces `PI_OFFLINE=1`, `PI_SKIP_VERSION_CHECK=1`, `PI_TELEMETRY=0`, and `NO_COLOR=1`. Secrets are never placed on argv or application logs.

The exact Pi 0.84.2 provider credential allowlist is:

```text
ANTHROPIC_AUTH_TOKEN ANTHROPIC_API_KEY ANTHROPIC_OAUTH_TOKEN
COPILOT_GITHUB_TOKEN ANT_LING_API_KEY OPENAI_API_KEY AZURE_OPENAI_API_KEY
DEEPSEEK_API_KEY NVIDIA_API_KEY GEMINI_API_KEY GOOGLE_CLOUD_API_KEY
GROQ_API_KEY CEREBRAS_API_KEY XAI_API_KEY FIREWORKS_API_KEY
TOGETHER_API_KEY BASETEN_API_KEY OPENROUTER_API_KEY AI_GATEWAY_API_KEY
ZAI_API_KEY ZAI_CODING_CN_API_KEY MISTRAL_API_KEY MINIMAX_API_KEY
MINIMAX_CN_API_KEY MOONSHOT_API_KEY OPENCODE_API_KEY KIMI_API_KEY
RADIUS_API_KEY HF_TOKEN CLOUDFLARE_API_KEY QWEN_TOKEN_PLAN_API_KEY
QWEN_TOKEN_PLAN_CN_API_KEY XIAOMI_API_KEY XIAOMI_TOKEN_PLAN_CN_API_KEY
XIAOMI_TOKEN_PLAN_AMS_API_KEY XIAOMI_TOKEN_PLAN_SGP_API_KEY
```

The exact provider configuration and ambient cloud-auth allowlist is:

```text
AZURE_OPENAI_BASE_URL AZURE_OPENAI_RESOURCE_NAME AZURE_OPENAI_API_VERSION
AZURE_OPENAI_DEPLOYMENT_NAME_MAP CLOUDFLARE_ACCOUNT_ID CLOUDFLARE_GATEWAY_ID
AWS_PROFILE AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN
AWS_BEARER_TOKEN_BEDROCK AWS_REGION AWS_DEFAULT_REGION AWS_ROLE_ARN
AWS_ROLE_SESSION_NAME AWS_CONTAINER_CREDENTIALS_RELATIVE_URI
AWS_CONTAINER_CREDENTIALS_FULL_URI AWS_CONTAINER_AUTHORIZATION_TOKEN
AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE AWS_WEB_IDENTITY_TOKEN_FILE
AWS_ENDPOINT_URL_BEDROCK_RUNTIME AWS_BEDROCK_FORCE_CACHE
AWS_BEDROCK_SKIP_AUTH AWS_BEDROCK_FORCE_HTTP1 GOOGLE_APPLICATION_CREDENTIALS
GOOGLE_CLOUD_PROJECT GCLOUD_PROJECT GOOGLE_CLOUD_LOCATION PI_CACHE_RETENTION
```

This list follows the pinned provider key map, provider implementations, and cloud-provider requirements. IRSA requires `AWS_ROLE_ARN` plus `AWS_WEB_IDENTITY_TOKEN_FILE`; `AWS_ROLE_SESSION_NAME` is optional. Vertex ADC requires its credentials path or default ADC under `HOME`, project, and location. No arbitrary environment variables are inherited.

Pi can read its selected settings, models, auth file, and allowlisted provider-auth environment and can contact the selected model provider for the disclosed turn. A credential whose `auth.json` key begins with `!command` causes Pi to execute that command through a shell and use its stdout, as documented by Pi 0.84.2. Session persistence, tools/bash, extensions, skills, prompt templates, themes, context files, approvals, telemetry, update checks, and startup network operations are disabled. These controls are a bounded execution profile, not proof that a trusted executable or provider is safe.

### Pi RPC contract

Commands and responses use correlated string IDs and strict LF-only JSONL. Pi RPC has no native output-schema command, so every initial and correction prompt ends with `OUTPUT_SCHEMA_JSON:` followed immediately by the exact compact mode-specific JSON schema. Codex continues using its native `outputSchema` request field.

Pi stdin, stdout, and stderr are nonblocking. State, prompt, final-text, abort, and turn deadlines include command writes; writes poll cancellation and their deadline instead of blocking cleanup on a child that stops reading. Records, the bounded protocol queue, and total accepted protocol output are capped at 2 MiB; assistant text is capped at 64 KiB. Prompt completion requires all of:

- a matching successful `prompt` response before events
- ordered `agent_start`, `turn_start`, correlated user message start/end, assistant message lifecycle, `turn_end`, and two-message user-plus-assistant `agent_end`, with no tool calls or tool results
- terminal `stopReason: "stop"` and no retry/queue continuation
- `agent_settled`
- a matching successful `get_last_assistant_text` response equal to the authoritative completed assistant message

The client rejects CRLF framing, malformed/unknown envelopes, unexpected or duplicate IDs, event reordering, tool/bash/extension-UI events, queue/retry/compaction continuations, provider changes, null or stale final text, non-stop completions, oversized records, aggregate/queue floods, EOF, and reader failures. Cancellation after prompt acceptance sends one correlated `abort`; the acknowledgement and `agent_settled` may arrive in either order, but both are required by the abort deadline. Failure kills and reaps the process group within bounded cleanup deadlines.

## Explicit Codex compatibility

Select Codex with `./interview --interviewer codex` or `INTERVIEW_TUTOR_INTERVIEWER=codex`. Install from a trusted source, authenticate with `codex login`, and confirm with `codex login status`. The executable resolves from `INTERVIEW_TUTOR_CODEX_EXECUTABLE` and then `PATH`.

Only exact Codex CLI versions 0.146.0 and 0.147.0 are accepted. The existing stable app-server subset remains unchanged: initialize/account reads, separate ephemeral interviewer/hinter threads, read-only sandbox, disabled sandbox network/web search, never-approve policy, strict numeric request correlation, typed request declines, terminal event matching, bounded stderr/JSONL queues, interrupt acknowledgement, process-group cleanup, and at most one replacement process for a later distinct operation. Unknown server requests fail closed. Codex receives no silent replay of a failed turn.

The Codex child environment is cleared and restores only `HOME`, `CODEX_HOME`, `PATH`, locale, proxy, and certificate variables. `OPENAI_API_KEY` and unrelated variables are excluded. Codex still reads its own account/configuration state and may use configured read-only tools or MCP servers to access other sandbox-readable paths. Prefer a dedicated minimal `CODEX_HOME` when that boundary is too broad.

The generated stable protocol evidence remains:

- 0.146.0 canonical schema SHA-256 `2f402b7d1356adccc1a4785c0656db457578ca9ea5d5b08953487a410c630ce8`
- 0.147.0 canonical schema SHA-256 `4422f141444d5531e549f4a3e8e7371c82e4dfbc6d5b6d06c8cd3dff8b4a8607`
- exact shared subset SHA-256 `d3187a04cbd0e7a46f3dda33934e1cfcb12415371fa2c0543663d216d71bafe5`

## Disclosure and memory-only state

After consent, Interview Tutor intentionally supplies exactly five application fields:

1. selected local statement
2. current source revision
3. most recent 16 KiB of local test output
4. bounded in-memory transcript
5. current user question

Questions are capped at 16 KiB. The transcript retains at most 128 entries and 256 KiB; each assistant response is at most 64 KiB. Hints omit transcript context, allow three levels per source revision, and must not reveal a complete solution. One malformed response receives one correction request in the same transport operation; a second malformed response fails closed.

Submission review starts only after local attempt recording succeeds and uses that submit operation's exact captured source, not a newer editor buffer. A response enters UI/transcript state only while operation, mode, and source revision still match. The local runner remains authoritative. Transcript state is cleared on reset, solve exit, and process exit; Interview Tutor writes no prompt or transcript log.

## Bounds and recovery

Version probes have a 10-second wall bound and combined 64-KiB output capture. Startup/state requests have 10-second bounds, turns 120 seconds, abort/interrupt acknowledgement 2 seconds, and shutdown 2 seconds followed by bounded kill/reap and reader drains. Temporary cwd removal failures are reported during explicit cleanup.

Authentication, executable, protocol, timeout, cancellation, and transport failures are typed rather than classified from message text. The UI shows backend-aware recovery guidance. Press `i` to reconnect for a later distinct operation, select `none`, or continue using local solve functions.

Automated tests use checked-in fake Pi RPC and Codex app-server executables and never invoke a live model or provider credentials.
