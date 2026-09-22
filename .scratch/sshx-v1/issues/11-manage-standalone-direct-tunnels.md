# 11: Manage standalone direct tunnels

**What to build:** Let a user start a direct service tunnel, close the launching terminal, then inspect, restart, or stop that tunnel from a later CLI process using an ID rather than a PID.

**Blocked by:** 07: Open a direct password shell without leaking secrets; 10: Select and open session service forwards atomically.

**Status:** resolved

- [x] Starting a standalone direct tunnel authenticates, establishes the complete requested forwarding set, and returns after the OpenSSH master detaches.
- [x] Registry and control directories are private, symlink-safe, permission-checked, and contain no password.
- [x] A later CLI process can list and query the tunnel by ID without relying on launcher memory.
- [x] Status distinguishes responsive master, bound listener, down state, and unknown application health.
- [x] Stop addresses only the protected control socket and never signals a recorded PID.
- [x] Restart is explicit stop-plus-start and no automatic reconnect occurs.
- [x] Repeating an exact active request returns the existing ID, while a different exact HostEntry never reuses it.
- [x] JSON and YAML lifecycle output use the same redacted versioned envelope as other management commands.

## Answer

9697a638359029f353dc9d192eedbb8bafa73428
