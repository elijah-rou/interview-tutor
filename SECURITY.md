# Security policy

## Supported scope

This project is pre-release. Security fixes target the latest revision on the default branch; older commits, forks, local modifications, third-party language adapters, and third-party Pi/Codex executables are not promised backports or support. Reports about documented trust boundaries are welcome, but a configured executable accessing permissions the user granted it is not by itself a containment bypass.

## Reporting

Report a suspected vulnerability privately through [GitHub private vulnerability reporting](https://github.com/elijah-rou/interview-tutor/security/advisories/new). Include the affected revision, Linux distribution, reproduction steps, expected/observed boundary, and the minimum non-secret logs needed to investigate. Do not open a public issue before coordinated disclosure.

Never paste API keys, login tokens, credential files, private solution content, or other personal data into a report. Redact paths and process output where they expose secrets. If a credential was disclosed, rotate/revoke it with its provider; deleting a report is not credential rotation.

## Credential and executable boundary

Interview Tutor does not parse, log, or persist provider credentials and does not call model-provider HTTP endpoints directly. The selected Pi or Codex executable owns provider authentication. Never paste credentials into the Interview composer.

The default `pi` from `PATH`, explicit `codex`, or a path selected through `INTERVIEW_TUTOR_PI_EXECUTABLE`/`INTERVIEW_TUTOR_CODEX_EXECUTABLE` is trusted user-configured code. Interview Tutor checks exact compatible versions, regular-file ownership, writable mode bits, and file identity before spawn. These checks establish compatibility and reduce replacement races; they do not prove provenance or make an untrusted executable safe.

Pi runs in a fresh empty mode-0700 cwd for every application turn with no session persistence. Its process environment is cleared and restored from a bounded locale/proxy/certificate/config/provider-auth allowlist. Tools/bash, extensions, skills, prompt templates, themes, context files, approvals, telemetry, update checks, and startup network operations are disabled. Pi still reads its selected configuration/auth inputs and contacts the selected model provider for the disclosed prompt. Pi 0.84.3 executes an `auth.json` credential beginning with `!command` through a shell and uses its stdout. Provider credentials are available to the trusted child where required but are never passed on argv.

Codex compatibility retains its empty cwd, bounded environment, ephemeral threads, read-only/no-network/never-approve request, and rejection of tool approvals. It can still use readable configuration, read-only tools, MCP servers, and local paths allowed by its sandbox/configuration. Neither transport is total process isolation. Use `--interviewer none` or legacy `--no-codex` when that boundary is unacceptable; disabled mode probes and spawns neither backend.

## Local solution privacy

Local catalog browsing, source editing, runner execution, SQLite progress, and attempts remain on the machine. Files are loaded/saved only at the catalog-planned regular source beneath the canonical project root. A local runner or custom adapter is executable code and should be reviewed before use.

After explicit backend-aware disclosure consent, the selected statement, current source, bounded latest test output, bounded memory-only transcript, and question are supplied to the configured process and may be sent to Pi's selected model provider or OpenAI through Codex under the user's account controls. Interview Tutor writes no transcript log and clears transcript state on reset, solve exit, and process exit. See [interviewer privacy details](docs/codex-compatibility.md) for exact transport, outbound, and readable-path boundaries.
