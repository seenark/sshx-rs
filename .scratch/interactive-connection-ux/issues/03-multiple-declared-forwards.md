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

## Comments

2026-10-01 fresh-target reconciliation: the integrated baseline (`artifact://1492`, `target/integration-20261001`) has 181/195 CLI tests passing, with three Pair Session failures assigned here. The current `target/ticket03-20261001` run passes all ten Pair CLI tests. Pair Session credentials now retain operation-local gateway/VM password attempts across failed startup and workspace retry; supplied descriptors are consumed once, without a global cache or password files. Completed Sessions opened from Hosts return directly to that same HostEntry with their non-secret result; focused `tui connect` keeps its workspace/retry behavior.

The realistic 80×24 Pair PTY tests prove nonzero VM shell exit restores Hosts with the exact VM ID and destination, both supplied passwords survive retry without terminal exposure, and custom local/remote/SOCKS requests fail before any master, runtime configuration, registry, or source mutation. A red-before/green-after regression also proves a rejected typed replacement can be corrected on the next submission; password prompting is limited per startup rather than for the retained credential's lifetime. Four existing root Hosts tests remove only the now-obsolete extra Esc after shell completion, preserve their identity, terminal-output, and cleanup assertions, and pass. Actual-binary PTY smokes confirm Pair shell exit 7 returns directly to Hosts with the exact VM identity, and a failed VM authentication can retry successfully with both supplied pipe passwords reused. Both smokes quit with 0, leave source bytes unchanged, expose no password, and create no Tunnel registry.

Parallel Standards and Spec reviews have zero remaining findings after correcting the prompted-password retry defect. Fixture/local validation only. `ready-for-agent` remains honest while the parent-owned integrated full-suite gate is pending. The broader root Hosts batch encountered system PTY exhaustion (`openpty` returned -1); the two affected output tests pass when run individually. Task-owned failed-test processes were cleaned up, and no unproved external process was stopped.

2026-10-02 final integrated reconciliation: `artifact://1785` passed all 197 CLI tests and every remaining suite except the direct Hosts Session test, which still required the obsolete post-shell connection workspace. The test now observes the immediate root Hosts return, exact selected secondary alias, destination, and HostEntry ID without an extra Esc or incidental result wording. It retains zero-forward runtime, owned-master closure, unchanged-source, and normal-quit checks, and additionally proves selection and review emit no runtime configuration before confirmation.

The corrected focused regression passes in `target/integration-20261002` (1 passed). An independent actual-binary PTY smoke confirms consent-gated startup, zero forwards, shell exit, restored `secondary` / `direct.example` / exact HostEntry ID, terminated owned master PID, unchanged source bytes, and quit status `0`. Fixture/local validation only. Status remains `ready-for-agent`: the parent-owned final serialized full-suite run is still required before `ready-for-human`.
