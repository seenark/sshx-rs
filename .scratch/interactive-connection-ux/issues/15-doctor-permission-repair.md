# 15: Diagnose problems and repair eligible permissions

**What to build:** A person can find blocking Doctor findings quickly in the TUI, inspect evidence and guidance, and deliberately apply only eligible private-permission repairs. CLI Doctor output remains useful and deterministic where required.

**Blocked by:** 01: Browse HostEntries and return from a Session.

**Status:** ready-for-agent

- [x] Findings are grouped by severity and stage in the TUI and human CLI output; each finding retains its evidence, affected path, and actionable guidance without repeated boilerplate.
- [ ] Doctor remains report-only until the user requests repair and confirms one plan listing eligible paths, existing modes, and target modes; each attempted repair has a visible result.
- [ ] Wrong-owner, symlink, shared-path, or otherwise ineligible targets are not changed. No host key is enrolled, and JSON/YAML schemas and non-interactive repair restrictions remain unchanged.

## Verification

- Isolated Doctor TUI smoke compiled `src/picker.rs` through a temporary external harness and opened it in a real terminal. Two config errors appeared under one `[error] config` heading, followed by `[warning] runtime`. The selected finding showed its path, local evidence, and guidance; Esc exited normally.
- `cargo test --offline --lib permissions::tests::apply_rejects_symlink_replacement_between_check_and_repair -- --exact` and `cargo test --offline --lib permissions::tests::apply_repairs_private_mode_and_skips_shared_file -- --exact` passed. An external Rust harness linked against `cargo build --offline --lib`: human Doctor output grouped severity/stage, retained path/evidence/guidance and the plan, left a mode-0644 fixture unchanged until `permissions::apply_all`, then set mode 0600 with one visible result.
- The targeted permission tests also passed for mode 000 and a file below a search-only directory on macOS. A Linux container probe confirmed `O_PATH` can traverse a search-only parent and `fchmodat` on `/proc/self/fd/<held-fd>` repairs a mode-000 file. Older-kernel `ENOSYS` and other Unix platform branches remain untested on those platforms.
- Full CLI/TUI Doctor repair flow and integration tests remain unverified: `src/main.rs` is a protected pre-existing tracked deletion, and `Cargo.toml` names it as the `sshx` binary entrypoint. Do not restore it merely to run tests. Once an entrypoint exists, verify plan review, one confirmation, result per attempted repair, non-interactive restrictions, eligibility, and unchanged JSON/YAML schemas; then run the full suite.
- The Doctor TUI workspace lives amid extensive pre-existing uncommitted edits to `src/picker.rs`. Do not stage unrelated picker changes merely to commit this ticket. Scoped standards and spec review found no issues in its grouping hunk; committed-diff review applies only to safely staged Doctor changes.
