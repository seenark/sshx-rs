# 06: Copy non-secret commands through explicit host actions

**What to build:** Let callers with an explicit HostEntry selector connect normally or copy an exact native SSH or sshx command through `--action`, using the platform clipboard when available and safe stdout fallback otherwise.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] `sshx connect SELECTOR --action connect|copy-ssh|copy-sshx` parses as one mutually exclusive action and skips any action menu.
- [x] With no `--action`, an explicit selector keeps existing immediate-connect behavior.
- [x] Direct `copy-ssh` produces a correctly POSIX-shell-quoted `ssh -F <config-root> <selected-alias>` command.
- [x] When several config roots reach the HostEntry, interactive execution shows scope, project, and path for root selection; execution without a TTY requires explicit disambiguation and never guesses.
- [x] `copy-sshx` produces a correctly quoted command that resolves the exact selected HostEntry, using stable identity plus config-root or source disambiguation when needed.
- [x] `copy-sshx` works for both direct and Pair-routed HostEntries; `copy-ssh` rejects Pair-routed HostEntries without fallback.
- [x] macOS uses `pbcopy`; Linux selects a compatible available backend from `wl-copy`, `xclip`, and `xsel`.
- [x] Clipboard tools are spawned without a shell and receive copied content through stdin, never argv.
- [x] Missing clipboard support writes only non-secret commands to stdout and returns an actionable result.
- [x] Unavailable actions fail without silently choosing another action.
- [x] `--action` combined with JSON or YAML output fails clearly and performs no action; existing machine-format inspection remains unchanged.

**Answer:** Added explicit connect actions with exact alias, identity, source, and config-root selection. `copy-sshx` stays noninteractive across multiple roots and preserves Pair discovery across cross-root routes. Clipboard backends receive command text on stdin with fixed native selection arguments; unavailable clipboard falls back to stdout without secrets.

**Evidence:** `cargo test --test cli` (64 passed); `cargo test --test tunnel` (3 passed); focused copy-action tests passed.
