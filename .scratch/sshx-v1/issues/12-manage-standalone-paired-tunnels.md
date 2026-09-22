# 12: Manage standalone paired tunnels

**What to build:** Let a user start and manage a standalone service tunnel through a Pair while `sshx` owns, reports, and cleans both the gateway transit master and VM service master transactionally.

**Blocked by:** 09: Open a paired VM shell in one terminal; 10: Select and open session service forwards atomically; 11: Manage standalone direct tunnels.

**Status:** ready-for-agent

- [ ] The registry records starting ownership before spawning either master and transitions to active only after both masters pass their documented checks.
- [ ] Concurrent identical starts reserve one request signature and produce one active tunnel ID.
- [ ] The idempotency signature includes exact physical entries, block fingerprints, Pair IDs, route, bind addresses, and effective forwards, but no secrets.
- [ ] VM startup failure stops only the newly created VM and gateway masters in reverse order and records cleanup failure honestly.
- [ ] A dead gateway or VM dependency makes composite status down without reconnecting or adopting another master.
- [ ] Stop closes the VM master before the gateway master and removes state only after ownership-safe control operations complete.
- [ ] Copied entries with identical aliases, destinations, or duplicated metadata never reuse each other's active tunnel.
- [ ] Delete is blocked while a proven managed tunnel uses either Pair entry and never stops the tunnel implicitly.
