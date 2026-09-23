# 02: Repair blocking permissions during ordinary commands

**What to build:** When an ordinary command is blocked by a safe permission problem, let the user repair that exact path in place and continue the original operation without leaving the command to remember or run `chmod` manually.

Type: task

Status: resolved

Blocked by: 01

- [x] A blocked command uses the shared permission-repair eligibility contract instead of defining a second ownership, type, symlink, or privacy policy.
- [x] The prompt shows the path, current mode, required mode, and reason before any mutation.
- [x] Consent applies the exact `0600` or `0700` repair and retries the blocked operation once.
- [x] A failed retry returns the real failure and never enters another repair loop.
- [x] Declining, `--no-input`, or a missing TTY performs no mutation and returns an actionable error.
- [x] Wrong ownership, symlinks, wrong path types, shared paths, and arbitrary parent directories are never repaired.
- [x] When manual repair is valid, the error includes a correctly shell-quoted `chmod` command.

## Answer

Runtime discovery now assesses a registered unreadable config root before content read, prompts once for an eligible owned file such as mode `000`, applies `0600`, and retries discovery once. Stored-password runtime checks use the same typed permission target and outcome contract; unsafe paths never mutate.

Evidence:

- `cargo test --test cli direct_connect_repairs_mode_zero_password_config_before_discovery_retry -- --exact` — passed.
- `cargo test --test doctor` — 12 passed.
- `cargo test --all-targets --all-features` — passed.
- `cargo check --all-targets`, clippy with `-D warnings`, and `cargo fmt --all -- --check` — passed.
