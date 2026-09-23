# 02: Repair blocking permissions during ordinary commands

**What to build:** When an ordinary command is blocked by a safe permission problem, let the user repair that exact path in place and continue the original operation without leaving the command to remember or run `chmod` manually.

**Blocked by:** 01: Repair private permissions through doctor.

**Status:** resolved

- [x] A blocked command uses the shared permission-repair eligibility contract instead of defining a second ownership, type, symlink, or privacy policy.
- [x] The prompt shows the path, current mode, required mode, and reason before any mutation.
- [x] Consent applies the exact `0600` or `0700` repair and retries the blocked operation once.
- [x] A failed retry returns the real failure and never enters another repair loop.
- [x] Declining, `--no-input`, or a missing TTY performs no mutation and returns an actionable error.
- [x] Wrong ownership, symlinks, wrong path types, shared paths, and arbitrary parent directories are never repaired.
- [x] When manual repair is valid, the error includes a correctly shell-quoted `chmod` command.

## Answer

Stored-password runtime creation now checks the source config through shared `permissions::assess`. Eligible `0600` repair prompts once, applies exact mode, then retries runtime creation once. Unsafe paths never mutate. Decline, `--no-input`, and no-TTY paths return a shell-quoted manual `chmod` command. Direct, paired, and tunnel runtime paths pass the input policy through.

Evidence:

- `cargo test --test cli` — 53 passed
- `cargo test --test doctor` — 12 passed
- `cargo test --test tunnel standalone_direct_tunnel_survives_launcher_and_stops_by_id` — passed
- `cargo check --all-targets`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo fmt --all -- --check`
