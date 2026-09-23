# 07: Choose host actions interactively and copy stored passwords safely

**What to build:** After selectorless `connect` chooses an exact HostEntry, let the user connect, copy an applicable command, or explicitly copy an existing stored password without leaking the secret through process or terminal output.

Type: task

Status: resolved

Blocked by: 03, 06

- [x] Selectorless `sshx connect` opens the fuzzy picker and then a host-action menu with Connect selected by default.
- [x] Selectorless `--action` still opens the picker but skips only the action menu.
- [x] Direct HostEntries offer Connect, Copy SSH, and Copy sshx; Copy password appears only when a non-empty stored password exists.
- [x] Pair-routed HostEntries offer only Connect and Copy sshx; explicitly requesting native SSH or password copy returns an actionable error.
- [x] `--action copy-password` errors when the selected direct HostEntry has no stored password.
- [x] Stored-password copy always displays the retention warning and requires immediate TTY confirmation, even when requested through the explicit action.
- [x] `--no-input` or a missing TTY always rejects password copy.
- [x] Passwords never appear in stdout, stderr, argv, environment variables, status text, or a generated plaintext `sshpass` command.
- [x] Password content reaches the native clipboard backend through stdin; a missing backend returns an actionable error and never falls back to stdout.
- [x] The UI does not claim automatic clipboard clearing and warns that clipboard-manager history may retain the secret.
- [x] Escape or Ctrl-C from the picker or action menu performs no connection or clipboard side effect, writes `Cancelled.` to stderr, and exits `130`.

## Answer

Added selectorless host-action menu with safe direct/Pair matrices. Added guarded stored-password copy with permission resolution before final warning and confirmation, native clipboard stdin only, no stdout fallback, and retention-history warning. Escape cancellation preserves side-effect-free exit `130`.

Evidence:

- `cargo test --test cli` (74 passed)
- `cargo test --test tunnel` (3 passed)
- `cargo test --lib` (15 passed)
- `cargo fmt --all -- --check`
- direct `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo check --all-targets`
