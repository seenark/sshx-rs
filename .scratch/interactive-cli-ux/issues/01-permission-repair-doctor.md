# 01: Repair private permissions through doctor

**What to build:** Let users review and repair every safe private-permission finding through `sshx doctor --fix-permissions`, while keeping ordinary `sshx doctor` report-only and preventing newly written private state from needing repair.

Type: task

Status: resolved

Blocked by: none (can start immediately)

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

Doctor now plans eligible owned unreadable registered config roots without reading their content first, including mode `000` files. One confirmation applies exact `0600`/`0700` repairs; report-only, non-interactive, ownership, symlink, wrong-type, and shared-path behavior stays unchanged.

Evidence:

- `cargo test --test cli doctor_fix_permissions_repairs_mode_zero_password_config_once -- --exact` — passed.
- `cargo test --test doctor` — 12 passed.
- `cargo test --all-targets --all-features` — passed.
- `cargo check --all-targets`, clippy with `-D warnings`, and `cargo fmt --all -- --check` — passed.
