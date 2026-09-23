# 03: Select connect targets with a live fuzzy picker

**What to build:** Replace the selectorless `connect` numbered prompt with a built-in live fuzzy picker that selects one exact HostEntry alias while preserving deterministic explicit selectors.

Type: task

Status: resolved

Blocked by: none (can start immediately)

- [x] `sshx connect` without a selector and with a usable TTY opens a live picker whose visible rows update as the query changes.
- [x] The picker renders one row per alias and shows destination, project and scope when present, plus source path and line number.
- [x] Selecting a secondary alias preserves that alias for the resulting OpenSSH invocation rather than falling back to the HostEntry's first alias.
- [x] Search covers alias, destination, and project, with alias exact matches, alias prefixes, fuzzy score, alias, source, and line used in the confirmed deterministic order.
- [x] An empty query sorts aliases case-insensitively with source and line tie-breakers.
- [x] Arrow keys move selection and Enter selects the highlighted row.
- [x] Escape or Ctrl-C writes `Cancelled.` to stderr, performs no connection or other side effect, and exits `130`.
- [x] Explicit selectors retain exact-match semantics and never fall back to fuzzy matching.
- [x] A required picker without a usable TTY returns an actionable selector-required error.
- [x] The picker is built into `sshx`, requires no external `fzf`, and stores no recent-host state.

## Answer

Implemented built-in raw-terminal picker in `src/picker.rs` and wired selectorless `connect` through it. Rows represent exact aliases, search alias/destination/project, rank exact aliases before prefixes and fuzzy matches, and use deterministic source tie-breakers. Picker preserves selected secondary aliases, handles arrows/Enter/backspace, and returns cancellation as exit `130` without connection side effects.

Evidence:

- `cargo test --test cli selectorless_connect_picker -- --nocapture` — 2 passed.
- `cargo test --bin sshx picker::tests -- --nocapture` — 3 passed, including destination-only/project-only queries and exact/prefix/tie ordering.
- `cargo check --all-targets` — passed.
