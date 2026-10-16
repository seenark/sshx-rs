# TUI redesign: modal, Helix-style sshx

Status: ready-for-agent
Type: task

## Problem Statement

sshx works, but day-to-day use is slow and hard to discover:

- After picking a HostEntry, the user is pushed through a port-binding step even when they only want a plain shell.
- Service forwards must be retyped every time, including the local port and `127.0.0.1`, although the bind address is almost always the same.
- Service forwards used last time are not offered again.
- Copying the SSH command, an sshpass command, or the stored password is buried.
- Creating, editing, and deleting HostEntries takes long question sequences followed by a separate review step.
- Keybindings and selection menus differ between screens, and there is no way to see which keys are available.
- Ports for a Pair can only be forwarded when declared beforehand, and declaring one needs two coordinated metadata lines.

The fuzzy finder and keeping sshx metadata as comments in SSH config files work well and must stay.

## Solution

A modal TUI in the style of Helix and Neovim:

- Enter on a HostEntry opens Session mode immediately, with no service forwards and no questions.
- Tab opens a ports screen. It is pre-filled with the HostEntry's declared services and the service forwards selected last time, so `⇥⏎` reconnects with the same forwards without typing.
- Input modes: Insert (typing goes to the search, the default), Normal (letters are commands, `space` is the leader), and Select (multi-select rows).
- A Helix-style which-key box appears on every prefix key, and `?` / F1 shows every key for the current screen. Both come from the same action registry that drives dispatch and the `:` command line.
- HostEntries are created and edited in a single form with a live preview of the exact block that will be written. Deletes are guarded, and undo/redo is available within the process.
- Copy actions sit under `y`. Password copies clear themselves from the clipboard after 30 seconds.
- Starting a standalone tunnel shows staged evidence ("master responsive", "listener ready") and resolves local port conflicts before the request is persisted.
- Any remote port can be forwarded on both direct and Pair routes. Declaring a service forward is an explicit save, and connecting never writes SSH config.
- Per-HostEntry usage state (last use, use count, last selected service forwards) lives in a sidecar file beside each config root (ADR 0004). It drives RECENT ordering and preselection.

## User Stories

### Connect

1. As an sshx user, I want Enter on a highlighted HostEntry to open Session mode with no service forwards, so that a plain shell takes zero extra prompts.
2. As an sshx user, I want typing `db` then Enter from launch to connect to the best match, so that the common case is three keystrokes.
3. As an sshx user, I want sshx to exit with ssh's exit code when the interactive session ends, so that sshx behaves like a shell command.
4. As an sshx user, I want a remote command's non-zero exit to be passed through silently, so that my own `exit 1` is not reported as a connection failure.
5. As an sshx user, I want a failure screen when a session-bound master fails to start, so that I learn what went wrong without the TUI disappearing.
6. As an sshx user with a Pair, I want the failure screen to name the failing hop (gateway or VM), so that I know which HostEntry to fix.
7. As an sshx user, I want the failure screen to show OpenSSH's stderr line, so that I see the real cause, such as `Permission denied (publickey)`.
8. As an sshx user, I want retry, edit host, copy an `ssh -v` command, and show full log on the failure screen, so that I can act on the failure immediately.

### Finder and main screen

9. As an sshx user, I want the existing fuzzy matching behavior kept, so that my muscle memory still works.
10. As an sshx user, I want an empty query to show a RECENT group first, then all HostEntries grouped by config root, so that what I use most is on top.
11. As an sshx user, I want fuzzy-score ties broken by use frequency, so that the host I use often wins over a rarely used one with the same score.
12. As an sshx user, I want a preview pane with route (direct, or gateway → VM), hostname, declared services, and last use, so that I can confirm the target before connecting.
13. As an sshx user, I want a `⇄N` badge with the number of declared services per HostEntry, so that I can see which hosts have services.
14. As an sshx user on a narrow terminal, I want the preview pane to collapse into a single-column list below a width threshold, so that the list stays readable.
15. As an sshx user with the same alias in several files, I want the existing duplicate-alias labels and source details kept, so that I can tell HostEntries apart.
16. As an sshx user, I want `j`/`k`, arrows, `gg`, `ge`, `^d`, `^u`, and counts such as `5j` in Normal input mode, so that navigation feels like Helix.
17. As an sshx user, I want `g r`, `g p`, `g w`, and `g t` to jump to recent, personal, work, and tunnels, so that I can move between groups quickly.
18. As an sshx user, I want `space f p|w|r` to filter by personal root, work root, or recent, so that I can narrow the list without typing.

### Input modes and discoverability

19. As an sshx user, I want Insert input mode on start, so that I can type a search right away.
20. As an sshx user, I want `esc` to switch from Insert to Normal input mode and never quit, so that pressing escape repeatedly is safe.
21. As an sshx user, I want `i`, `a`, or `/` to return to Insert input mode, so that switching back is familiar.
22. As an sshx user, I want `q` or `:q` to quit from Normal input mode and Ctrl+C to quit from any input mode, so that leaving is always possible.
23. As an sshx user, I want a status line with input mode, active config-root filter, position/total, number of running standalone tunnels, and pending keys, so that I always know the TUI's state.
24. As an sshx user, I want a which-key box to open immediately on `space`, `g`, `y`, `d`, `c`, and `'`, so that I never need to memorize sequences.
25. As an sshx user, I want which-key entries that do not apply to the current row dimmed, so that I can see why an action is unavailable.
26. As an sshx user, I want `esc` or backspace to go up one level in which-key, so that I can back out of a submenu.
27. As an sshx user who knows the keys, I want `:set autoinfo=false` to hide the which-key box, so that it stops covering the list.
28. As an sshx user, I want `?` in Normal input mode (or in Insert with an empty query) and F1 anywhere to open help, so that help is always one key away.
29. As an sshx user, I want help grouped by section, filterable with fuzzy search, and listing the current screen's section first, so that I find keys fast.
30. As a maintainer, I want help, which-key, dispatch, and `:` completion to read one action registry, so that the documented keys can never drift from real behavior.

### Ports screen and service forwards

31. As an sshx user, I want Tab (in either input mode) to open the ports screen for the highlighted HostEntry, so that service forwards are opt-in.
32. As an sshx user, I want the ports screen to list declared services plus the service forwards I selected last time, so that I don't retype them.
33. As an sshx user, I want last time's selection pre-checked, so that `⇥⏎` reconnects with the same service forwards.
34. As an sshx user, I want `x` to toggle a row, so that I can pick which service forwards to open.
35. As an sshx user, I want `o` to add a row using the `REMOTE[=LOCAL]` grammar (`5432`, `80=3000`), so that adding an ad-hoc service forward is short.
36. As an sshx user, I want the bind address to default to `127.0.0.1` (setting `bind`) and never be typed, so that the common case has no noise.
37. As an sshx user, I want rows shown as `localhost:3000 → 80` with the service name when declared, so that I can read them at a glance.
38. As an sshx user with a Pair, I want ad-hoc service forwards to work through the VM like on direct routes, so that Pairs need no prior declaration.
39. As an sshx user, I want `s` on a row to save it as a declared service in the HostEntry's metadata, so that keeping a service forward is a deliberate action.
40. As an sshx user, I want `dd` on an ad-hoc row to remove the row, so that I can drop a mistake.
41. As an sshx user, I want `dd` on a declared service to remove its declaration (undoable with `u`), so that I can clean up metadata from the TUI.
42. As an sshx user, I want Enter on the ports screen to open Session mode with the checked service forwards, so that connect-with-forwards is one key.
43. As an sshx user, I want `space t t` on the ports screen to start a standalone tunnel with the checked service forwards, so that I can choose Tunnel mode from the same screen.
44. As an sshx user, I want existing preflight checks (duplicate local ports, busy ports, SOCKS duplicates) to flag rows before connecting, so that conflicts surface early.
45. As an sshx user, I want connecting never to modify my SSH config, so that routine use leaves my files untouched.

### Copy

46. As an sshx user, I want `yy` to copy the ssh command, including `-L` for checked service forwards when coming from the ports screen, so that I can paste it elsewhere.
47. As an sshx user with a Pair, I want `yy` to copy the sshx form of the command, so that the copied command reproduces the Pair route.
48. As an sshx user with a stored password on a direct HostEntry, I want `yp` to copy a ready-to-run sshpass command, so that I can connect from another terminal.
49. As a security-conscious user, I want `yp` to copy a command that starts with a space and passes the password through the `SSHPASS` environment variable with `sshpass -e`, so that the password stays out of argv and, when the shell ignores space-prefixed lines, out of history.
50. As an sshx user, I want `yp` greyed out for Pairs and for HostEntries without a stored password, so that I understand why it is unavailable.
51. As an sshx user, I want `yw` to copy the stored password and `yh` to copy the HostName, so that common values are one chord away.
52. As an sshx user, I want `yi` to copy the selected standalone tunnel's ID, so that I can use it with CLI commands.
53. As an sshx user, I want a status-line toast such as `✓ copied ssh command`, so that I know the copy succeeded.
54. As a security-conscious user, I want passwords never rendered on screen, and masked as `••••` in previews, so that shoulder-surfing reveals nothing.
55. As an sshx user, I want the clipboard-history warning shown only on my first password copy, so that later copies are frictionless.
56. As a security-conscious user, I want copied passwords and sshpass commands cleared from the clipboard after 30 seconds, even after sshx has exited, so that secrets don't linger.
57. As an sshx user, I want auto-clear to clear the clipboard only if it still holds what sshx copied, so that it never wipes something I copied afterwards.
58. As an sshx user, I want `:set clipboard_clear=off` and `:set password_confirm=true` to change those defaults, so that I can tune the friction.

### Host create, edit, delete, undo

59. As an sshx user, I want `o` to open a single-screen form for a new HostEntry, so that I don't answer a question sequence.
60. As an sshx user, I want only alias and host required, with user and port showing their defaults, so that a minimal host takes seconds.
61. As an sshx user, I want a `via` field with fuzzy search over existing HostEntries that pairs the HostEntry with that gateway after a review, so that jump hosts are easy to set and sshx can connect through them. (Revised: `via` first wrote `ProxyJump`, which direct connect rejects with `UNSUPPORTED_DIRECTIVE`.)
61a. As an sshx user, I want a VM whose loopback `HostName:Port` matches one gateway `LocalForward` in the same file marked `↳? gateway`, with `Enter` offering to pair and connect (`y`), connect directly (`d`), or cancel (`n`), and `c p` to pair without connecting, so that I never have to remember which gateway a VM needs.
62. As an sshx user editing a gateway or VM, I want a read-only `pair: gateway → VM` line instead of `via`, so that I don't break the Pair route.
63. As an sshx user, I want a `pass` field that stores the password through the existing password-safety checks, so that insecure files are refused.
64. As an sshx user, I want a `file` field to pick the config root or included file to write into, so that hosts land where I keep them.
65. As an sshx user, I want a live preview of the exact block that will be written, comments included, so that no separate review step is needed.
66. As an sshx user, I want `j`/`k` between fields, `i` to edit, and `:w`, `:q`, `:wq`, `ZZ` to save or cancel, so that the form feels like an editor.
67. As an sshx user, I want `cc` to edit the highlighted HostEntry in the same form, so that edits and creation work the same way.
68. As an sshx user, I want edits to rewrite only the HostEntry's own block and keep unknown directives and comments, so that my hand-written config survives.
69. As an sshx user, I want `ce` to open `$EDITOR` at the HostEntry's line and rediscover afterwards, so that I can make complex edits by hand.
70. As an sshx user, I want any sshx edit of a HostEntry without `##SSHX ID` to add one, so that its usage state survives later renames.
71. As an sshx user, I want `dd` on a HostEntry to show its block and ask y/N, so that I don't delete by accident.
72. As an sshx user, I want deleting a gateway or VM refused, with an offer to dissolve the Pair first, so that Pairs never end up half-deleted.
73. As an sshx user, I want deleting a HostEntry used by a running standalone tunnel refused, so that I don't orphan a tunnel.
74. As an sshx user, I want `u` and `^r` to undo and redo create, edit, delete, and declare/undeclare changes made in this sshx process, so that mistakes are cheap.
75. As an sshx user, I want undo refused when the file changed on disk since sshx wrote it, so that undo never overwrites someone else's edit.
76. As a security-conscious user, I want undo history kept in memory only, so that no extra copies of files containing passwords appear on disk.

### Standalone tunnels

77. As an sshx user, I want starting a standalone tunnel to show staged evidence per hop and per listener ("master responsive", "listener ready"), so that I see progress rather than a silent wait.
78. As an sshx user, I want a busy local port to offer the next free port, another port, or skip, before anything starts, so that one conflict does not abort the tunnel.
79. As an sshx user, I want the persisted tunnel request to contain the ports I finally chose, so that the registry matches what is running.
80. As an sshx user, I want `space t l` or `g t` to list standalone tunnels with ID, HostEntry, age, service forwards, master and listener evidence, so that I can see what is running.
81. As an sshx user, I want Enter on a tunnel to open SSH through its master, `dd` to stop it, and `r` to restart it with the same service forwards, so that I can manage tunnels in place.
82. As an sshx user, I want `space t x` to stop all standalone tunnels, so that I can clean up quickly.
83. As an sshx user, I want status words limited to "master responsive" and "listener ready" with evidence, so that sshx never claims application health.

### Command line and Select input mode

84. As an sshx user, I want a `:` command line with Tab completion for `:q`, `:w`, `:wq`, `:ssh <host> [REMOTE[=LOCAL]…]`, `:tunnel start|list|stop <id>`, `:set <option>[=value]`, `:root add|remove <path>`, and `:help`, so that every action is also a command.
85. As an sshx user, I want `V` to enter Select input mode and mark several HostEntries, so that I can act on many at once.
86. As an sshx user, I want `y` in Select input mode to copy one ssh command per selected row, so that I can paste them into scripts.
87. As an sshx user, I want `d` in Select input mode to delete the selected rows with one combined confirmation, applying the same Pair and tunnel refusals, so that bulk cleanup stays safe.
88. As an sshx user, I want `space t t` in Select input mode to start a standalone tunnel per selected row with its last selected service forwards, so that I can bring up several tunnels at once.

### First run and settings

89. As a new user with no registered config roots, I want sshx to detect candidate SSH config files, let me tick them and label each personal or work, and then go straight to the finder, so that setup takes one screen.
90. As an sshx user, I want `:set` to change `startmode`, `autoinfo`, `bind`, `clipboard_clear`, and `password_confirm`, persisted to my sshx settings, so that preferences survive restarts.
91. As an sshx user, I want a non-loopback `bind` value refused just as forwards are today, so that the setting can't silently expose listeners.

### Usage state

92. As an sshx user, I want usage state stored beside each config root, so that it syncs with my SSH config through my existing private git.
93. As an sshx user, I want a read-only config root to just skip recording usage, so that connecting still works.
94. As an sshx user, I want deleting the usage sidecar to lose only ordering and preselection, so that it is never a source of truth for hosts.

### CLI compatibility

95. As a script author, I want existing CLI flags (`--action connect|copy-ssh|copy-sshx|copy-password`, `--forward`, `-L`, `-R`, `-D`) to keep working, so that my scripts don't break.
96. As a script author, I want `--forward REMOTE[=LOCAL]` to accept undeclared remote ports on both direct and Pair routes, so that the CLI matches the TUI.
97. As a script author, I want custom `-L` specifications with an arbitrary bind or destination to stay direct-route only, so that Pair transit routes remain safe.

## Implementation Decisions

### Vocabulary and ADRs

- Use the `CONTEXT.md` glossary terms: HostEntry, Pair, gateway, VM, Session mode, Tunnel mode, standalone tunnel, session-bound master, config root, filesystem source of truth, service forward, declared service, usage state, input mode, "listener ready", "master responsive".
- "Mode" on its own always means Session mode or Tunnel mode. Insert/Normal/Select are input modes.
- ADR 0001 (OpenSSH engine, filesystem source of truth, tunnel ownership) and ADR 0003 (directive pass-through) stay in force.
- ADR 0004 (usage state beside config roots) is accepted and clarifies ADR 0001 decision 2.

### Keymap engine (new module, foundation)

- A pure, terminal-free module that maps key sequences to actions, per input mode and per screen.
- Built as a trie of key sequences. It tracks pending keys and a numeric count prefix, which applies to movement only. Counts on destructive commands are out of scope.
- One action registry holds, for every action, its key sequences, its `:` command name, its help section, its description, and an availability predicate for the current row (which drives dimming).
- Dispatch, which-key, help, and `:` completion all read this registry. No screen keeps a hand-written key list.
- Interface shape: feed one key event, and get back one of: an action (with count), "pending" (with the which-key entries for the current prefix), or "unmapped". Also query the entries for a prefix and the completions for a command-line prefix.
- The existing hand-rolled byte decoder and its 30 ms escape poll stay. It gains F1 and any other codes the keymap needs. Ctrl+Space is not used.

### Screen modules

- The current single picker module (all TUI screens, about 4.9k lines) is split into a TUI module directory, one module per screen, with each screen's unit tests beside it. The first ticket does this split with no behavior change.
- Screens: main finder with preview, ports, copy toasts, host form, delete confirmation, failure, tunnel progress, tunnel list, help, command line, first run.
- The fuzzy matcher (score, rank, row comparison) and its tests are kept unchanged. Only presentation changes, plus a frequency tie-break input from usage state.
- Duplicate-alias labelling and source details are kept.
- The preview pane collapses below a fixed width threshold of 70 columns.

### Service forward model (behavior change)

- Using a forward needs no declaration. `REMOTE[=LOCAL]` targets `127.0.0.1` on the route's last hop and works on direct and Pair routes. A declared service only provides defaults: local port, destination host, and name. The "service not declared" error is removed.
- The declaration is a single self-declaring metadata line: `##SSHX SERVICE <remote> [LOCAL=<local>] [HOST=<host>] [NAME=<name>]`.
  - The rule that `##SSHX SERVICE` must match a `##PORT` line is removed.
  - `NAME=` is a new attribute.
  - Existing `##PORT N` lines are still read as shorthand for `##SSHX SERVICE N`. sshx never writes `##PORT`.
  - Existing field aliases and positional parsing stay readable. sshx writes only the canonical form above.
- Custom `-L` specifications with an arbitrary bind address or destination remain direct-route only.
- Saving a declared service (`s`) and removing one (`dd` on a declared row) go through the reviewed mutation engine and are undoable.
- Connecting never writes SSH config.
- CLI help text for `--forward` changes from "Forward declared service" to describe any remote port, with declared services supplying defaults.

### Usage state (new module, ADR 0004)

- One sidecar file per config root, beside the root file, named after it in the same way as the existing mutation lock: `.<root-name>.sshx-usage.json`, with private file mode.
- Per HostEntry it holds: last use time, use count, and last selected service forwards (remote, local, and declared-or-ad-hoc). It holds no other host fields, and discovery never reads hosts from it.
- Key: `##SSHX ID` when present, otherwise first alias plus source path relative to the root. When sshx mutates a HostEntry that lacks an ID, it adds one and moves the usage record to the new key.
- A HostEntry in an included file belongs to the first root that includes it.
- Written after a session-bound master or standalone tunnel starts successfully. If the write fails, sshx skips it silently and the connection is unaffected.
- No sync handling. The user syncs the sidecar with their private git.

### Copy

- Clipboard backends stay as they are (pbcopy, wl-copy, xclip, xsel).
- `yy`: the ssh command for direct HostEntries, the sshx form for Pairs (today's copy-sshx behavior), plus checked service forwards when invoked from the ports screen.
- `yp`: direct HostEntries with `##PASSWORD` only. The copied text is a space, then `SSHPASS='<password>' sshpass -e ssh …`, with the password shell-quoted.
- `yw` copies the password, `yh` the HostName, and `yi` the tunnel ID.
- Defaults: `password_confirm=false` and `clipboard_clear=30`. The clipboard-history warning appears once, and its acknowledgment is persisted in settings.
- Auto-clear runs as a detached helper so it survives sshx exiting. After the delay it reads the clipboard back and clears it only if a hash of the content still matches. If the backend can't read back, it does not clear.
- Passwords are never rendered. Previews show `••••`.

### Host form, delete, undo

- The form drives the existing reviewed mutation engine (create, update, delete plans). The live preview replaces the separate mutation review screen for this path. The engine underneath is unchanged.
- `via` names a Pair gateway, prefilled with the suggested gateway. `:w` writes the other field changes, then opens the Pair review, which applies the existing journaled Pair setup mutation. `via` is hidden on Pair members, which show a read-only Pair line instead. The form no longer writes `ProxyJump`; `ce` edits it by hand.
- Password storage keeps the existing file-mode check.
- Edits rewrite only the HostEntry's byte span.
- `ce` launches `$EDITOR +<line> <file>` and rediscovers when it exits.
- Delete is refused for Pair members, with an offer to run the existing Pair delete mutation first. It is also refused while a standalone tunnel uses the HostEntry (existing active-use check).
- Undo/redo: an in-memory stack of before/after byte pairs per write, for mutations made in this process (create, edit, delete, declare, undeclare). Before applying, undo checks the existing snapshot fingerprint (length, mtime, digest) and refuses if the file changed. No backup files are written. Pair mutations are not on the undo stack, because they keep their own journal and recovery.

### Tunnels

- Port-conflict choices (next free port, another port, skip) are made during preflight, before the tunnel request is written to the registry. The persisted request therefore holds the final ports, as ADR 0001 §5 requires.
- The progress screen shows preflight results first, then real evidence per master and per listener.
- The tunnel list replaces the current tunnels screen.
- Ownership checks are unchanged: locked registry plus control socket, never a PID alone.

### Failure handling

- The failure screen appears only when a session-bound master fails to start. sshx then knows the failing hop and has its stderr.
- After the interactive session ends, sshx exits with ssh's exit code and does not return to the finder.

### Settings

- Settings move to schema version 2. New optional fields with defaults: `startmode` (insert), `autoinfo` (true), `bind` (127.0.0.1), `clipboard_clear` (30), `password_confirm` (false), plus the password-warning acknowledgment.
- Version 1 files load with default preferences and are rewritten as version 2 on the next save. Older sshx binaries refuse version 2 files; the owner accepted this.

### First run

- When no config roots are registered, the first-run screen lists detected SSH config files with personal/work labels, then opens the finder. It replaces the current setup screen for the first-run case only.

## Testing Decisions

- A good test drives sshx the way a user does and asserts what the user or OpenSSH can observe:
  - argv and runtime config passed to fake `ssh` and `sshpass`
  - bytes of SSH config files before and after
  - the usage sidecar's contents
  - clipboard contents captured by a fake backend
  - exit codes
  - visible screen text

  Tests do not assert internal state, private helper calls, or source text.
- **Seam 1, the binary in a PTY (primary).** Use the existing integration harness: PTY-driven interactions, isolated `HOME`, and fake `ssh`/`sshpass`/paired `ssh`/clipboard backends on `PATH`. Every user-visible story is tested here:
  - `db⏎` reaches the shell with zero prompts.
  - `db⇥⏎` uses last-selected service forwards with no typing.
  - Pair ad-hoc service forwards appear in the VM runtime config.
  - `s`/`dd` declare and undeclare with exact config bytes, and `u` reverts them.
  - Undo is refused after an external edit.
  - Delete is refused for Pair members and in-use HostEntries.
  - `yp` text form, and its unavailability for Pairs.
  - Auto-clear clears only matching content.
  - The failure screen appears on master-start failure, and a remote non-zero exit passes through.
  - Tunnel port-conflict substitution is persisted in the registry.
  - The usage sidecar is written on success and skipped on a read-only root.
  - CLI `--forward` works with undeclared ports on both routes, and custom `-L` is still rejected on Pairs.
- **Seam 2, the keymap engine interface (unit).** Pure tests with no terminal:
  - key sequence → action per input mode and screen
  - pending state and which-key entries for each prefix
  - count prefixes on movement
  - `esc`/backspace going up a level
  - availability-driven dimming
  - every registry action reachable from which-key or help (acceptance: nothing undiscoverable)
  - `:` completion
- Prior art:
  - PTY scenarios with fake OpenSSH in the integration suite: connect, setup, tunnel lifecycle, password retry, clipboard copy with a fake pbcopy/wl-copy.
  - Unit tests beside the fuzzy matcher and key decoder.
  - Service metadata parser tests in the session module.
  - Mutation engine tests for recovery and snapshot refusal.
- Existing integration tests and picker unit tests must keep passing. Tests that move with the screen split move with their screen.
- Delete tests that pin the old behavior instead of re-pinning it: the port-binding step after selection, `FORWARD_NOT_FOUND` for undeclared ports, and "service metadata references undeclared remote port".
- No ratatui `TestBackend` rendering tests. Layout is checked through visible text in PTY scenarios.

## Out of Scope

- Named port sets (`SET=`, save/manage sets, delete warnings about sets).
- Marks (`m`, `'`).
- `relativenumber`.
- `.` repeat.
- Counts on destructive commands (`3dd`).
- OSC 52 clipboard fallback.
- Light/dark themes. The terminal's default colors are used.
- sshpass copy for Pairs.
- Undo for Pair mutations, and undo across sshx processes.
- Returning to the finder after a session ends.
- Settings downgrade support for older sshx binaries.
- Any change to CLI flags other than `--forward` accepting undeclared ports.

## Further Notes

- Acceptance for the whole effort:
  - From launch, `db⏎` reaches an interactive shell with zero extra prompts.
  - From launch, `db⇥⏎` connects with the last-selected service forwards and no typing of ports or bind address.
  - Every action is reachable from which-key or `?`.
  - No status text claims application health.
  - `cargo test` passes.
  - No ADR is silently overridden.
- That ad-hoc service forwards are safe on Pair routes is an inference: the VM runtime config already receives declared services in the same shape. The forward-model work must prove it with a paired fake-OpenSSH scenario before the ports screen depends on it.
- The owner chose the settings version bump to 2 after implementation started on version 1.
- The sidecar is synced through the owner's private git. Simultaneous use on two machines can produce merge conflicts in that file, and the only loss is ordering. ADR 0004 records this.
- Tickets are not split yet. When they are, they go under this directory's `issues/` per the issue-tracker conventions.
