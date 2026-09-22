# 09: Open a paired VM shell in one terminal

**What to build:** Let a user select a paired VM once, authenticate the gateway and VM independently, and receive the VM shell in the current terminal through an isolated temporary transit route.

**Blocked by:** 07: Open a direct password shell without leaking secrets; 08: Create and validate a one-to-one Pair.

**Status:** claimed

- [ ] The gateway master opens one app-selected loopback transit port without changing source LocalForward, HostName, or Port values.
- [ ] The VM master connects to that same temporary port with its own authentication and known-hosts identity.
- [ ] Gateway and VM may use different passwords or keys, and each failure is attributed to the correct role and stage.
- [ ] VM HostKeyAlias derives from the immutable VM ID and remains stable when the temporary port changes.
- [ ] A local gateway listener is reported only as transit readiness; successful VM master authentication is the route-success proof.
- [ ] The VM shell opens through the VM master with inherited terminal streams and no second terminal tab.
- [ ] Two concurrent sessions of the same Pair use distinct ports and control directories, and closing one does not affect the other.
- [ ] Normal and handled-signal cleanup stops the VM master before the gateway master and never targets external SSH processes.
