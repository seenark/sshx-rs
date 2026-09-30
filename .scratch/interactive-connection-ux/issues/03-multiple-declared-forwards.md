# 03: Select several declared service forwards in one workspace

**What to build:** A person can choose several services declared on one HostEntry, edit their local ports together, review every mapping, and start one Session or standalone Tunnel. A Pair uses only services declared on its VM.

**Blocked by:** 01: Browse HostEntries and return from a Session; 02: Continue incomplete CLI operations in the TUI.

**Status:** ready-for-agent

- [x] One connection workspace shows mode, exact HostEntry or Pair route, several editable service-forward rows, and a single review/confirmation without replacing the whole screen for each field.
- [x] The example server ports 5432, 6379, and 3001 can map respectively to local ports 5432, 6378, and 3001 in one request; repeated declared-service CLI options express the same request.
- [x] Session mode permits zero forwards and returns to the TUI after its shell ends; Tunnel mode requires at least one, returns an ID, and leaves the TUI open. Pair requests use only VM-declared services.
- [x] Duplicate or unavailable local listeners are reported against the affected rows before startup; cancellation starts no master or listener. Startup errors clean up owned resources.

## Verification

Focused fixture/local validation passes: nine CLI tests cover three edited Session rows at 18×12, three-row Tunnel startup/returned ID/registry/stop, Pair VM-only restrictions, unchecked edit cancellation, listener conflicts, startup rollback with preserved checked rows, and repeated declared-service CLI forwards. The corrected rollback comparison passes separately; seven declared-service library tests and the existing three-forward Pair Tunnel ownership/reverse-cleanup test pass. Actual-binary terminal smoke shows the exact `5432=5432`, `6379=6378`, `3001=3001` rows together with source identity, and rejects an empty Tunnel without changing mode; both cancellations exit `130`. No user-server validation is claimed.

The previous `verified` status is withdrawn because the observed full suite (`artifact://1035`) failed ticket-related PTY scenarios and other-ticket CLI/Tunnel scenarios. Obsolete banner gates and incidental wording assertions were removed, not re-pinned. Direct Hosts selections now use the shared connection workspace. Required integrated full-suite validation remains with the parent agent; keep `ready-for-agent` until that run passes.

Two-axis review of `d6a5ff7...f2070fc`: no documented-standard violations or spec findings; one nonblocking test-fixture duplication observation. Shared test and documentation commits include only reviewed ticket-related content; unrelated working changes remain unstaged.

Reconciliation against the current workspace passes seven focused CLI tests: edited three-row Session at 18×12, three-row standalone Tunnel with ID/list/stop, Pair VM-only rows, affected-row listener conflicts, checked/unchecked rollback and resource release, repeated CLI forwards, and cancelled unchecked editing. Ticket03 interactions now wait on the declared service identity rather than the obsolete workspace-banner helper path; rollback also proves review has not started a master or emitted a runtime config. An actual-binary PTY smoke shows the three checked server/local mappings (5432/5432, 6379/6378, 3001/3001), exact source, and one review; cancellation exits `130` before launch. Fixture/local evidence only.

The named custom-row, remote-row, and SOCKS-row scenarios belong to tickets04–06 and remain unchanged here; their row-retention/startup-failure coverage is not deleted. The latest full suite (`artifact://1191`) still has 51 CLI failures across other ticket scopes. The parent owns the next integrated full-suite run; that passing gate remains required before `ready-for-human`.

Parallel Standards and Spec reviewers found zero findings in the owned reconciliation patch against `8e29f94`. Only the three semantic interaction-gate replacements, the pre-confirmation rollback assertions, and this tracked ticket update are included; unrelated `tests/cli.rs` work remains unstaged.
