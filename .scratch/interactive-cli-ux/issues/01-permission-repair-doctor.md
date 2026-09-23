# 01: Repair private permissions through doctor

**What to build:** Let users review and repair every safe private-permission finding through `sshx doctor --fix-permissions`, while keeping ordinary `sshx doctor` report-only and preventing newly written private state from needing repair.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] One shared eligibility contract accepts only current-user-owned paths of the expected regular-file or directory type, rejects symlinks and shared paths, and never changes ownership or arbitrary parent directories.
- [x] Eligible files are repaired to exactly `0600`; eligible directories are repaired to exactly `0700`.
- [x] Generic readable SSH files without a stored password do not become findings only because other users can read them.
- [x] `sshx doctor` remains report-only and performs no permission mutation.
- [x] `sshx doctor --fix-permissions` displays one complete plan and asks for one confirmation before changing any path.
- [x] Non-interactive execution never applies repairs.
- [x] Repair continues after individual failures and reports `fixed`, `skipped`, and `failed` for every planned path.
- [x] Doctor exits nonzero when any repair fails or an unsafe permission finding remains.
- [x] New private files are created as `0600` and new private directories as `0700`, including application settings regardless of process `umask`.

## Answer

Implemented in backend commits `36eabf8` and `8e61e8e`, then wired in CLI.

- `cargo test --test cli doctor_fix_permissions` — 3 passed.
- `cargo test --test doctor` — 12 passed.
- `cargo test --test tunnel standalone_direct_tunnel_survives_launcher_and_stops_by_id` — passed.
- `cargo check --all-targets` — passed.
- `cargo fmt --all -- --check` — passed.

Ordinary `doctor` stays report-only. `doctor --fix-permissions` prompts once in a TTY, skips without a TTY, and emits one machine-readable report with per-path results.
