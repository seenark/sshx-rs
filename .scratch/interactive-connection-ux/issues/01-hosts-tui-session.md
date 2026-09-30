# 01: Browse HostEntries and return from a Session

**What to build:** A person can launch sshx on a usable terminal, search exact HostEntries in a persistent Hosts TUI, open a direct Session without forwards, and return to the selected HostEntry after the shell exits. This is a complete, usable first path through the new UI, not an empty application shell.

**Blocked by:** None (can start immediately).

**Status:** ready-for-human

- [x] A usable terminal opens a keyboard-accessible Hosts view; non-interactive bare invocation prints CLI help instead. Search preserves secondary aliases, deterministic ranking, and visible source identity for duplicate aliases.
- [x] A selected direct HostEntry opens a Session through the existing OpenSSH engine; closing the shell restores the same HostEntry in the TUI and cleans up its session-bound master.
- [x] A narrow or colorless terminal still exposes selected identity, available action, focus, and cancellation. Leaving the TUI without opening a Session creates no connection or mutation.

**Verification:** `cargo test --offline --test hosts` passes 29 tests; `cargo test --offline --bin sshx picker::tests::` passes 14 tests. The repaired PTY regression navigates Connection review/confirmation, proves no premature OpenSSH startup, opens a zero-forward Session using `secondary`, observes owned-master cleanup, and returns to the same selected HostEntry ID without source mutation. Continuation cancellation checks alias/source/ID instead of incidental route wording. Separate real-binary fixture smoke proves complete CLI connection bypasses TUI, and 48×12 colorless bare Hosts retains identity, action, focus, and cancellation without extra OpenSSH or mutation. Nested Standards and Spec reviews report zero findings. Fixture/local validation only; no user-server validation claimed.

**Remaining:** No ticket01 implementation blocker. The coordinating agent owns end-of-effort full-suite integration; existing failures in `artifact://1125` were not rerun merely to confirm them. Unrelated dirty `tests/cli.rs` remains untouched. Human validation on an authorized real server remains separate.
