# 09: Complete nested help for HostEntry and Pair workflows

**What to build:** Extend command-aware help across every HostEntry and Pair group, leaf, and accepted alias so users can discover selection, mutation, and Pair behavior without external documentation.

**Blocked by:** 04: Use the fuzzy picker for every interactive HostEntry selection; 08: Provide nested help for root and top-level workflows.

**Status:** claimed

- [ ] Help exists for `host`, `host list`, `host show`, `host create`, `host update`, `host rename`, and `host delete` through all three accepted help forms.
- [ ] Help exists for `pair`, `pair setup`, `pair create`, `pair list`, and `pair validate` through all three accepted help forms.
- [ ] Group pages list their accepted subcommands and guide users to leaf help.
- [ ] Leaf pages document applicable selectors, exact-match behavior, interactive picker fallback, prompts, conflicts, examples, exit behavior, and related commands.
- [ ] Pair setup and its create alias document interactive gateway and VM selection accurately.
- [ ] Alias help identifies canonical behavior without rejecting an otherwise accepted spelling.
- [ ] Successful help uses deterministic plain stdout and exit `0`; parse errors use stderr, exit `2`, and the nearest relevant usage.
- [ ] Examples contain no real passwords and use canonical HostEntry, Pair, gateway, and VM terminology.
