# 10: Complete nested help for every tunnel path and alias

**What to build:** Extend command-aware help across direct, paired, and compatibility tunnel spellings so each accepted start, list, status, stop, and restart path explains its actual standalone tunnel behavior.

**Blocked by:** 04: Use the fuzzy picker for every interactive HostEntry selection; 08: Provide nested help for root and top-level workflows.

**Status:** ready-for-agent

- [ ] Help exists for the `tunnel`, `tunnel direct`, and `tunnel paired` groups through all three accepted help forms.
- [ ] Help exists for every accepted `start`, `list`, `status`, `stop`, and `restart` spelling under the root tunnel group and its direct or paired groups.
- [ ] Group pages list accepted subcommands and identify canonical spellings where aliases share behavior.
- [ ] Start pages document exact selectors, fuzzy picker fallback, forwarding options, password input semantics, route constraints, and cancellation behavior where applicable.
- [ ] Lifecycle pages document tunnel identifiers, ownership expectations, output, significant errors, and exit behavior.
- [ ] Alias help describes the behavior accepted at that alias instead of returning generic root usage.
- [ ] Successful help uses deterministic plain stdout and exit `0`; parse errors use stderr, exit `2`, and the nearest relevant usage.
- [ ] Examples contain no real passwords and use canonical standalone tunnel, Pair, gateway, and VM terminology.
