# 01: Browse HostEntries and return from a Session

**What to build:** A person can launch sshx on a usable terminal, search exact HostEntries in a persistent Hosts TUI, open a direct Session without forwards, and return to the selected HostEntry after the shell exits. This is a complete, usable first path through the new UI, not an empty application shell.

**Blocked by:** None (can start immediately).

**Status:** ready-for-human

- [x] A usable terminal opens a keyboard-accessible Hosts view; non-interactive bare invocation prints CLI help instead. Search preserves secondary aliases, deterministic ranking, and visible source identity for duplicate aliases.
- [x] A selected direct HostEntry opens a Session through the existing OpenSSH engine; closing the shell restores the same HostEntry in the TUI and cleans up its session-bound master.
- [x] A narrow or colorless terminal still exposes selected identity, available action, focus, and cancellation. Leaving the TUI without opening a Session creates no connection or mutation.

**Verification:** Shared Hosts dependencies committed in `dadd673`. On this committed source baseline, `cargo check --offline --bin sshx` passes; `cargo test --offline --bin sshx picker::tests::` passes 15 tests; `cargo test --offline --test hosts` passes 3 PTY tests. The PTY test covers bare/non-TTY help, explicit non-TTY error, secondary-alias selection, direct Session return, and session-bound master cleanup. Earlier PTY smoke covered duplicate alias source identity, stale-source rejection, empty first-run Hosts, and narrow colorless cancellation without connection. No user-server validation claimed.

**Remaining:** Full-suite verification awaits separate repair of unrelated uncommitted `tests/cli.rs` compile errors (missing imports and invalid `concat!` argument). Human validation on a real server remains. Shared integration committed previously uncommitted picker/session dependencies; Ctrl+T/S/D now route to Tunnels/Setup/Doctor workspaces. No third implementation attempt was needed.
