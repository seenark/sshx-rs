# 01: Browse HostEntries and return from a Session

**What to build:** A person can launch sshx on a usable terminal, search exact HostEntries in a persistent Hosts TUI, open a direct Session without forwards, and return to the selected HostEntry after the shell exits. This is a complete, usable first path through the new UI, not an empty application shell.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [x] A usable terminal opens a keyboard-accessible Hosts view; non-interactive bare invocation prints CLI help instead. Search preserves secondary aliases, deterministic ranking, and visible source identity for duplicate aliases.
- [x] A selected direct HostEntry opens a Session through the existing OpenSSH engine; closing the shell restores the same HostEntry in the TUI and cleans up its session-bound master.
- [x] A narrow or colorless terminal still exposes selected identity, available action, focus, and cancellation. Leaving the TUI without opening a Session creates no connection or mutation.

**Verification:** Reconciled the Hosts PTY regression with the shared Connection workspace: selecting `secondary` starts no OpenSSH; review starts no OpenSSH; explicit confirmation opens a zero-forward direct Session; shell completion cleans the owned master; Esc returns to the selected `secondary` row and the same HostEntry ID. The test also proves unchanged source bytes. The exact-selector continuation regression now checks the selected alias, source line, and HostEntry ID rather than incidental route wording. `cargo test --offline --test hosts` passes all 29 tests; `cargo test --offline --bin sshx picker::tests::` passes all 14 selection tests. Separate real `target/debug/sshx` smoke with an isolated fixture and fake external OpenSSH proves complete `connect secondary --no-input` runs directly, produces shell output, has zero forwards, and cleans its master. A bare-invocation smoke at 48×12 with `NO_COLOR=1` and `TERM=dumb` proves visible selection/action/cancellation, secondary-alias search, exact identity via Tab, exit 0, no additional OpenSSH calls, and unchanged config. Fixture/local validation only; no user-server validation claimed.

**Remaining:** Required two-axis nested review is pending. The unchanged full-suite failures recorded in `artifact://1125` were not rerun merely to confirm them; the coordinating agent owns the end-of-effort full-suite run. Unrelated dirty `tests/cli.rs` remains untouched. Human validation on an authorized real server remains separate.
