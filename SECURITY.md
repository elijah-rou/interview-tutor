# Security policy

## Supported scope

This project is pre-release. Security fixes target the latest revision on the default branch; older commits, forks, local modifications, third-party language adapters, and third-party Codex executables are not promised backports or support. Reports about documented trust boundaries are welcome, but a configured executable accessing permissions the user granted it is not by itself a containment bypass.

## Reporting

Report a suspected vulnerability privately through [GitHub private vulnerability reporting](https://github.com/elijah-rou/interview-tutor/security/advisories/new). Include the affected revision, Linux distribution, reproduction steps, expected/observed boundary, and the minimum non-secret logs needed to investigate. Do not open a public issue before coordinated disclosure.

Never paste API keys, login tokens, credential files, private solution content, or other personal data into a report. Redact paths and process output where they expose secrets. If a credential was disclosed, rotate/revoke it with its provider; deleting a report is not credential rotation.

## Credential and executable boundary

Interview Tutor does not accept or read OpenAI API keys, Codex login tokens, or credential files, and it does not call OpenAI HTTP endpoints directly. The configured Codex CLI owns its authentication through `HOME`/`CODEX_HOME`. Use `codex login` and `codex login status`; do not paste credentials into the Interview composer.

The default `codex` from `PATH`, or a path selected through `INTERVIEW_TUTOR_CODEX_EXECUTABLE`, is trusted user-configured code. Interview Tutor checks exact compatible versions, regular-file ownership, writable mode bits, and file identity before spawn. These checks establish compatibility and reduce replacement races; they do not prove provenance or make an untrusted executable safe. Install Codex from a trusted source and prefer a dedicated minimal profile/home.

The app-server requests read-only/no-network/never-approve operation and rejects tool approval requests. This is not total process isolation. The selected process can still use readable configuration, read-only tools, MCP servers, and other local paths allowed by its sandbox/configuration. Run with `--no-codex` when that boundary is unacceptable.

## Local solution privacy

Local catalog browsing, source editing, runner execution, SQLite progress, and attempts remain on the machine. Files are loaded/saved only at the catalog-planned regular source beneath the canonical project root. A local runner or custom adapter is executable code and should be reviewed before use.

After explicit Codex disclosure consent, the selected statement, current source, bounded latest test output, bounded memory-only transcript, and question are supplied to the configured Codex process and may be sent to OpenAI under the user's account controls. Interview Tutor writes no transcript log and clears transcript state on reset, solve exit, and process exit. See [Codex privacy details](docs/codex-compatibility.md) for the exact outbound and readable-path boundaries.
