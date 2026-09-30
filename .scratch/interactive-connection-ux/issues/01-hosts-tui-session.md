# 01: Browse HostEntries and return from a Session

**What to build:** A person can launch sshx on a usable terminal, search exact HostEntries in a persistent Hosts TUI, open a direct Session without forwards, and return to the selected HostEntry after the shell exits. This is a complete, usable first path through the new UI, not an empty application shell.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [x] A usable terminal opens a keyboard-accessible Hosts view; non-interactive bare invocation prints CLI help instead. Search preserves secondary aliases, deterministic ranking, and visible source identity for duplicate aliases.
- [x] A selected direct HostEntry opens a Session through the existing OpenSSH engine; closing the shell restores the same HostEntry in the TUI and cleans up its session-bound master.
- [x] A narrow or colorless terminal still exposes selected identity, available action, focus, and cancellation. Leaving the TUI without opening a Session creates no connection or mutation.

**Verification:** Dirty-worktree fixture/local validation: `cargo test --offline --bin sshx picker::tests::` (15 passed) and `cargo test --offline --test hosts hosts_opens_direct_session_and_restores_selected_alias -- --exact` (passed). The executable PTY test covers bare/non-TTY help, explicit non-TTY error, secondary-alias selection, direct Session return, zero forwards, and session-bound master cleanup. Earlier PTY smoke also covered duplicate alias source identity, stale-source rejection, empty first-run Hosts, and narrow colorless cancellation without connection. No user-server validation claimed.

**Remaining:** `HEAD` calls `picker::HostsState` and `picker::browse_hosts`, but those APIs exist only in shared uncommitted `src/picker.rs`. That file also contains unfinished work for other tickets and depends on uncommitted `src/session.rs` forwarding APIs and `Cargo.toml`/`Cargo.lock` dependencies; committing it wholesale would include unrelated work. A clean checkout therefore cannot compile the executable Hosts path. Split and commit a self-contained picker dependency without other-ticket changes, then verify the committed tree. Full-suite verification is blocked by pre-existing uncommitted `tests/cli.rs` compile errors (missing imports and invalid `concat!` argument); repair those separately and run the full suite once. Review also found that advertised Ctrl+T/S/D shortcuts exit instead of opening Tunnels/Setup/Doctor; those workspaces remain outside this ticket's direct Session path. Keep this ticket ready-for-agent until its committed dependency and full verification are complete.
