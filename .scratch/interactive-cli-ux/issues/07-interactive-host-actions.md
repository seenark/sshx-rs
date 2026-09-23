# 07: Choose host actions interactively and copy stored passwords safely

**What to build:** After selectorless `connect` chooses an exact HostEntry, let the user connect, copy an applicable command, or explicitly copy an existing stored password without leaking the secret through process or terminal output.

**Blocked by:** 03: Select connect targets with a live fuzzy picker; 06: Copy non-secret commands through explicit host actions.

**Status:** ready-for-agent

- [ ] Selectorless `sshx connect` opens the fuzzy picker and then a host-action menu with Connect selected by default.
- [ ] Selectorless `--action` still opens the picker but skips only the action menu.
- [ ] Direct HostEntries offer Connect, Copy SSH, and Copy sshx; Copy password appears only when a non-empty stored password exists.
- [ ] Pair-routed HostEntries offer only Connect and Copy sshx; explicitly requesting native SSH or password copy returns an actionable error.
- [ ] `--action copy-password` errors when the selected direct HostEntry has no stored password.
- [ ] Stored-password copy always displays the retention warning and requires immediate TTY confirmation, even when requested through the explicit action.
- [ ] `--no-input` or a missing TTY always rejects password copy.
- [ ] Passwords never appear in stdout, stderr, argv, environment variables, status text, or a generated plaintext `sshpass` command.
- [ ] Password content reaches the native clipboard backend through stdin; a missing backend returns an actionable error and never falls back to stdout.
- [ ] The UI does not claim automatic clipboard clearing and warns that clipboard-manager history may retain the secret.
- [ ] Escape or Ctrl-C from the picker or action menu performs no connection or clipboard side effect, writes `Cancelled.` to stderr, and exits `130`.
