# 08: Create and validate a one-to-one Pair

**What to build:** Let a user pair one exact gateway entry with one exact VM entry, recording stable identities and an unambiguous transit destination that remains inspectable after external edits.

**Blocked by:** 06: Update, rename, and delete HostEntries losslessly.

**Status:** ready-for-agent

- [ ] Pair setup selects exact gateway and VM HostEntries rather than resolving by alias alone.
- [ ] Missing IDs are assigned transactionally and every paired ID is globally unique and immutable.
- [ ] A transit destination is inferred only when the VM port matches exactly one gateway LocalForward candidate.
- [ ] Zero, duplicate, or conflicting candidates require explicit transit host and port instead of guessing.
- [ ] Pair setup rejects route-altering ProxyCommand or ProxyJump directives that conflict with the approved route.
- [ ] One gateway entry cannot be paired to multiple VM entries, while distinct gateway entries may reach the same physical gateway.
- [ ] Copied or malformed IDs are reported and never regenerated or merged silently.
- [ ] External deletion or mutation produces a broken-reference diagnostic, and deleting either referenced entry is blocked without cascade.
