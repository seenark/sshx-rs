# 03: Select several declared service forwards in one workspace

**What to build:** A person can choose several services declared on one HostEntry, edit their local ports together, review every mapping, and start one Session or standalone Tunnel. A Pair uses only services declared on its VM.

**Blocked by:** 01: Browse HostEntries and return from a Session; 02: Continue incomplete CLI operations in the TUI.

**Status:** ready-for-agent

- [ ] One connection workspace shows mode, exact HostEntry or Pair route, several editable service-forward rows, and a single review/confirmation without replacing the whole screen for each field.
- [ ] The example server ports 5432, 6379, and 3001 can map respectively to local ports 5432, 6378, and 3001 in one request; repeated declared-service CLI options express the same request.
- [ ] Session mode permits zero forwards and returns to the TUI after its shell ends; Tunnel mode requires at least one, returns an ID, and leaves the TUI open. Pair requests use only VM-declared services.
- [ ] Duplicate or unavailable local listeners are reported against the affected rows before startup; cancellation starts no master or listener. Startup errors clean up owned resources.

## Verification

The latest full suite (`artifact://1035`) failed ticket-related PTY scenarios, so the previous `verified` status is withdrawn. Reconciliation is in progress: verify declared rows, direct and Pair routes, cancellation, listener conflicts, and startup cleanup against the actual binary before marking complete. Project-wide validation remains coordinated by the parent agent after integration.
