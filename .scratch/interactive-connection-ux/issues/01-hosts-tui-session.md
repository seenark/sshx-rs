# 01: Browse HostEntries and return from a Session

**What to build:** A person can launch sshx on a usable terminal, search exact HostEntries in a persistent Hosts TUI, open a direct Session without forwards, and return to the selected HostEntry after the shell exits. This is a complete, usable first path through the new UI, not an empty application shell.

**Blocked by:** None (can start immediately).

**Status:** ready-for-human

- [x] A usable terminal opens a keyboard-accessible Hosts view; non-interactive bare invocation prints CLI help instead. Search preserves secondary aliases, deterministic ranking, and visible source identity for duplicate aliases.
- [x] A selected direct HostEntry opens a Session through the existing OpenSSH engine; closing the shell restores the same HostEntry in the TUI and cleans up its session-bound master.
- [x] A narrow or colorless terminal still exposes selected identity, available action, focus, and cancellation. Leaving the TUI without opening a Session creates no connection or mutation.

**Verification:** Second-attempt reconciliation preserves all ten ticket01 CLI regressions: duplicate/secondary alias and exact ID restoration, Include/source/Pair gateway freshness, Unicode search, closed/redirected stdout, supplied-password invalidation, unsupported Match rejection, and malformed unused service metadata. `cargo test --offline --test cli hosts_tui_ -- --nocapture` passes 19 tests; the malformed-metadata zero-forward Session and complete direct CLI regression each pass. The strengthened exact-ID regression passes separately. `cargo test --offline --test hosts` passes 29 tests; `cargo test --offline --bin sshx picker::tests::` passes 14 tests. Actual-binary fixture smoke on a 48×18 colorless terminal proves zero-forward Session review/confirmation, no premature OpenSSH startup, redirected shell output reaching the terminal, exact secondary alias/ID restoration, owned-master cleanup, and unchanged sources. Complete CLI retains stdout and bypasses TUI. Fixture/local validation only; no user-server validation claimed.

**Remaining:** No ticket01 implementation blocker. Nested Standards and Spec reviews each report zero findings. The coordinating agent owns the shared end-of-effort full-suite gate; `artifact://1380` is the pre-repair failure baseline and was not rerun merely to confirm reported failures. Only ticket01-owned CLI functions and necessary runtime code are included; unrelated dirty changes remain untouched. Human validation on an authorized real server remains separate.
