# 04: Use the fuzzy picker for every interactive HostEntry selection

**What to build:** Give tunnel, host mutation, and Pair workflows the same exact-alias live picker as `connect`, including new interactive gateway and VM selection for Pair creation.

**Blocked by:** 03: Select connect targets with a live fuzzy picker.

**Status:** ready-for-agent

- [ ] Direct and paired tunnel starts use the shared picker whenever a HostEntry selector is omitted and input is interactive.
- [ ] `host update`, `host rename`, and `host delete` use the shared picker whenever they require interactive HostEntry selection.
- [ ] `pair setup` and its `pair create` alias can select both gateway and VM interactively instead of rejecting missing selectors.
- [ ] Every flow preserves the exact alias row and HostEntry identity selected by the user, including duplicate and secondary aliases.
- [ ] Non-connect flows continue directly into their requested operation and never show the connect host-action menu.
- [ ] Explicit selectors remain exact and keep their existing non-interactive behavior.
- [ ] Escape or Ctrl-C performs no mutation, connection, Pair creation, or tunnel start and exits `130`.
