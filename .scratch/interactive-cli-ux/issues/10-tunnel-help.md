# 10: Complete nested help for every tunnel path and alias

**What to build:** Extend command-aware help across direct, paired, and compatibility tunnel spellings so each accepted start, list, status, stop, and restart path explains its actual standalone tunnel behavior.

**Blocked by:** 04: Use the fuzzy picker for every interactive HostEntry selection; 08: Provide nested help for root and top-level workflows.

**Status:** resolved

- [x] Help exists for the `tunnel`, `tunnel direct`, and `tunnel paired` groups through all three accepted help forms.
- [x] Help exists for every accepted `start`, `list`, `status`, `stop`, and `restart` spelling under the root tunnel group and its direct or paired groups.
- [x] Group pages list accepted subcommands and identify canonical spellings where aliases share behavior.
- [x] Start pages document exact selectors, fuzzy picker fallback, forwarding options, password input semantics, route constraints, and cancellation behavior where applicable.
- [x] Lifecycle pages document tunnel identifiers, ownership expectations, output, significant errors, and exit behavior.
- [x] Alias help describes the behavior accepted at that alias instead of returning generic root usage.
- [x] Successful help uses deterministic plain stdout and exit `0`; parse errors use stderr, exit `2`, and the nearest relevant usage.
- [x] Examples contain no real passwords and use canonical standalone tunnel, Pair, gateway, and VM terminology.

**Answer:** Added deterministic plain-text help routing and pages for every tunnel group, start mode, lifecycle operation, and direct/paired alias. Help documents exact selectors, source and Host line disambiguation, fuzzy picker fallback, forwarding and password descriptors, Pair and ProxyCommand boundaries, tunnel ownership, stable IDs, lifecycle output, cancellation, errors, and exit behavior.

**Evidence:** `cargo test --test cli` (84 passed); `cargo fmt --all -- --check`; `cargo check --all-targets`; `cargo clippy --all-targets --all-features -- -D warnings`.
