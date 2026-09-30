# 14: Register config roots from the TUI

**What to build:** A first-time user with no usable HostEntries can reach Setup, inspect registered config roots, and add a valid personal or work root without maintaining a second HostEntry database.

**Blocked by:** 01: Browse HostEntries and return from a Session; 02: Continue incomplete CLI operations in the TUI.

**Status:** ready-for-agent

- [x] An empty Hosts view provides a clear route to Setup and Doctor; Setup lists current roots and can select scope, project, and a real SSH config file.
- [x] Explicit CLI setup inputs are prefilled when interactive continuation is needed; a fully specified CLI setup retains its existing direct behavior.
- [x] Invalid paths do not register. Registering a root requires an intentional action and preserves SSH config files as the filesystem source of truth.

## Comments

- 2026-09-30: Existing `src/picker.rs` contains a Setup workspace and Hosts shortcuts for Setup and Doctor, but `src/main.rs` is deleted in the pre-existing working tree. That entrypoint owns CLI setup, TUI dispatch, and registration. Restoring the protected deletion would overwrite unrelated work. No end-to-end acceptance criterion or runtime verification is complete. Restore or replace the entrypoint while preserving that work, connect Setup to the existing `settings` registration path, then verify empty Hosts navigation, CLI prefill/direct behavior, and invalid-path/cancellation safety.

- 2026-09-30: `src/picker.rs` still provides the Setup form and empty-Hosts shortcuts, and `src/settings.rs` still provides root registration primitives. Existing CLI/PTY tests cover empty-Hosts registration, prefill, direct setup, invalid paths, and cancellation, but none can verify the missing binary: `Cargo.toml` targets the protected, pre-existing deletion of `src/main.rs`. Do not restore it or copy its historical content to another entrypoint. Remaining: reconcile that deletion with its owner, wire Setup dispatch/validation/registration in the intended entrypoint, run the targeted CLI/PTY scenarios and required review, then check criteria and advance status.

- 2026-09-30: The earlier missing-entrypoint comments are superseded: `src/main.rs` is restored in the current branch. Setup now shares CLI root validation/registration, preserves supplied fields and existing roots, focuses a missing path, rejects missing/directory/symlink root files, and registers only on Ctrl+S. Empty Hosts reaches Setup and Doctor with Ctrl+S/Ctrl+D. A scoped work request does not silently register an unrelated personal default. Complete CLI requests remain direct; successful TUI registration returns to Hosts.

- 2026-09-30: Fixture/local verification: `cargo build --bin sshx` passed. All 13 exact ticket CLI/PTY scenarios passed against an isolated HEAD-based CLI test image, which compiled independently of the unrelated dirty working-tree suite. Actual xterm PTY runs showed registered roots and supplied fields, no settings write while editing a valid root, exact work registration on Ctrl+S, unchanged SSH config, Hosts remaining open, and cancellation exit 130 with saved roots unchanged. Non-TTY, `--no-input`, JSON missing-root requests and directory roots returned exit 2 without registration. A complete CLI request on a real TTY exited 0 without input and saved the exact supplied root. Two review regressions failed before and passed after fixes: an explicit work-scoped config stays visible in Hosts after Setup; a stale saved root displays its discovery error alongside success instead of closing the TUI. Both fixes were also exercised on the actual xterm surface.

- 2026-09-30: Implementation commits: `63e5dbc`, `27d180e`. Required parallel Standards and Spec reviews are complete: no hard standards violations or remaining actionable spec findings; the inherited PTY lifecycle duplication is a nonblocking judgment call. Required full-suite verification remains blocked by prior evidence: unrelated dirty `tests/cli.rs` has an invalid `concat!` and missing `fs`/`OpenOptions` imports (589 compile errors); it was preserved and the known failure was not rerun. A broad scoped filter also exposed two inherited Pair-test failures (`pair_setup_picker_selects_gateway_and_vm_alias_rows`, `pair_setup_requires_explicit_transit_for_ambiguous_candidates`); these are not Ticket14 verification. Status stays ready-for-agent only because required repository verification remains blocked. The verified ticket criteria are checked above. No user-server validation is claimed.
