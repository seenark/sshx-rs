# 12: Manage standalone paired tunnels

**What to build:** Let a user start and manage a standalone service tunnel through a Pair while `sshx` owns, reports, and cleans both the gateway transit master and VM service master transactionally.

**Blocked by:** 09: Open a paired VM shell in one terminal; 10: Select and open session service forwards atomically; 11: Manage standalone direct tunnels.

**Status:** resolved

- [x] The registry records starting ownership before spawning either master and transitions to active only after both masters pass their documented checks.
- [x] Concurrent identical starts reserve one request signature and produce one active tunnel ID.
- [x] The idempotency signature includes exact physical entries, block fingerprints, Pair IDs, route, bind addresses, and effective forwards, but no secrets.
- [x] VM startup failure stops only the newly created VM and gateway masters in reverse order and records cleanup failure honestly.
- [x] A dead gateway or VM dependency makes composite status down without reconnecting or adopting another master.
- [x] Stop closes the VM master before the gateway master and removes state only after ownership-safe control operations complete.
- [x] Copied entries with identical aliases, destinations, or duplicated metadata never reuse each other's active tunnel.
- [x] Delete is blocked while a proven managed tunnel uses either Pair entry and never stops the tunnel implicitly.

## Answer

b3d05ea3002e7850e16af7aaecbdff1ce66275ea
