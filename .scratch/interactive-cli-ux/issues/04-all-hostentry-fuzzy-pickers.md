# 04: Use the fuzzy picker for every interactive HostEntry selection

**What to build:** Give tunnel, host mutation, and Pair workflows the same exact-alias live picker as `connect`, including new interactive gateway and VM selection for Pair creation.

Type: task

Status: resolved

Blocked by: 03

- [x] Direct and paired tunnel starts use the shared picker whenever a HostEntry selector is omitted and input is interactive.
- [x] `host update`, `host rename`, and `host delete` use the shared picker whenever they require interactive HostEntry selection.
- [x] `pair setup` and its `pair create` alias can select both gateway and VM interactively instead of rejecting missing selectors.
- [x] Every flow preserves the exact alias row and HostEntry identity selected by the user, including duplicate and secondary aliases.
- [x] Non-connect flows continue directly into their requested operation and never show the connect host-action menu.
- [x] Explicit selectors remain exact and keep their existing non-interactive behavior.
- [x] Escape or Ctrl-C performs no mutation, connection, Pair creation, or tunnel start and exits `130`.

## Answer

The shared HostEntry picker now accepts an action label and preserves selected aliases. Direct and paired tunnel starts, all host mutations, and both Pair setup selections use labeled pickers. Pair planning receives selected gateway and VM aliases, so secondary aliases identify the exact mutation blocks. `pair setup` and `pair create` accept zero or one positional selector and fill missing roles interactively. Cancellation exits `130` before operation side effects. Non-TTY errors name the requested action instead of claiming `connect`.

Evidence:

- `cargo test --test cli` — 57 passed
- `cargo test --test tunnel` — 3 passed
- `cargo check --all-targets`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo fmt --all -- --check`
