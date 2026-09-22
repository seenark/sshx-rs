# 11: Manage standalone direct tunnels

**What to build:** Let a user start a direct service tunnel, close the launching terminal, then inspect, restart, or stop that tunnel from a later CLI process using an ID rather than a PID.

**Blocked by:** 07: Open a direct password shell without leaking secrets; 10: Select and open session service forwards atomically.

**Status:** ready-for-agent

- [ ] Starting a standalone direct tunnel authenticates, establishes the complete requested forwarding set, and returns after the OpenSSH master detaches.
- [ ] Registry and control directories are private, symlink-safe, permission-checked, and contain no password.
- [ ] A later CLI process can list and query the tunnel by ID without relying on launcher memory.
- [ ] Status distinguishes responsive master, bound listener, down state, and unknown application health.
- [ ] Stop addresses only the protected control socket and never signals a recorded PID.
- [ ] Restart is explicit stop-plus-start and no automatic reconnect occurs.
- [ ] Repeating an exact active request returns the existing ID, while a different exact HostEntry never reuses it.
- [ ] JSON and YAML lifecycle output use the same redacted versioned envelope as other management commands.
