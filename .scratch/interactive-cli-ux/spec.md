# Interactive CLI UX specification

Status: confirmed

## Purpose

Improve command discoverability and interactive host workflows without changing the core boundaries defined by the OpenSSH engine, filesystem source of truth, HostEntry identity, or Pair routing.

## Current behavior

- Help is limited to `Usage: sshx [--version]` and only works when `-h` or `--help` is the first argument.
- Interactive HostEntry selection uses a numbered list and one substring query.
- `doctor` reports unsafe permissions but never offers a permission repair.
- Direct session compilation copies `ProxyCommand`, but source validation rejects OpenSSH `%` tokens. Direct standalone tunnels and Pair routes reject `ProxyCommand`.
- Selecting a HostEntry for `connect` starts the connection immediately. There is no clipboard integration or host action menu.

## Goals

1. Provide complete, nested help at every accepted command path.
2. Offer explicit, safe permission repair instead of requiring users to remember `chmod` syntax.
3. Replace every interactive HostEntry selector with one built-in live fuzzy picker.
4. Support native `ProxyCommand` behavior for direct sessions and direct standalone tunnels.
5. Let users connect or copy an applicable command or stored password after interactive selection, with direct CLI actions for automation-safe cases.

## Non-goals

- Full OpenSSH wildcard, global directive, or `Host *` inheritance.
- `ProxyJump` support.
- `ProxyCommand` inside Pair routes.
- An external `fzf` runtime dependency.
- Usage history or a second host database.
- Clipboard support outside macOS and Linux release targets.
- Copying a plaintext `sshpass` command.
- Non-interactive stored-password copying.
- Ownership repair, symlink repair, or changes to shared paths.

## Terminology

Use `config root`, `filesystem source of truth`, `HostEntry`, `Pair`, `gateway`, and `VM` as defined in `CONTEXT.md`.

## Rich nested help

### Accepted forms

Every accepted command or command group supports:

```text
sshx <command-path> --help
sshx <command-path> -h
sshx help <command-path>
```

No arguments display root help. `sshx help` also displays root help.

Help flags are position-sensitive in the standard CLI manner:

- `sshx --help host list` displays root help.
- `sshx host --help list` displays `host` group help.
- `sshx host list --help` displays `host list` help.
- `sshx help host list` displays `host list` help.

### Help paths

Help exists for the root, every group, every canonical leaf, and every accepted alias:

- Root: `sshx`
- Top-level leaves: `setup`, `doctor`, `connect`
- Host group: `host`, `host list`, `host show`, `host create`, `host update`, `host rename`, `host delete`
- Pair group: `pair`, `pair setup`, `pair create`, `pair list`, `pair validate`
- Tunnel groups: `tunnel`, `tunnel direct`, `tunnel paired`
- Tunnel leaves: `tunnel start`, `tunnel direct start`, `tunnel paired start`, plus each accepted `list`, `status`, `stop`, and `restart` spelling under `tunnel`, `tunnel direct`, and `tunnel paired`

An alias help page may identify its canonical spelling, but it must document the behavior accepted at that alias.

### Required content

Each page contains applicable sections in this order:

1. Name and one-sentence purpose
2. Usage forms
3. Positional arguments
4. Options, including defaults, conflicts, and whether a prompt can supply a missing value
5. Subcommands for group pages
6. At least one valid example for a leaf page
7. Exit behavior and significant error conditions
8. Related commands when useful

Help uses canonical glossary terms. Examples never contain real passwords.

### Rendering contract

- Plain text only; no color and no pager.
- Successful help writes to stdout and exits `0`.
- Help is deterministic when redirected or snapshot-tested.
- Parse errors remain on stderr and exit `2`, with the nearest relevant usage summary.
- `--format` does not alter help rendering.

## Permission repair

### Safe candidate

A path is eligible only when all conditions hold:

- The current effective user owns the path.
- The path is the expected regular file or directory type.
- No path component being repaired is a symlink.
- The path is known to be private: sshx-managed state, known-hosts state, a password-bearing SSH source file, or a private config path whose owner permissions block the requested operation.
- Repair does not require changing ownership or modifying a shared path.

Generic readable SSH files with no stored password do not become repair findings merely because other users can read them. Arbitrary parent directories are never changed.

### Target modes

- Eligible files are set to exactly `0600`.
- Eligible directories are set to exactly `0700`.

These modes preserve read/write access for the owner. Directory mode also preserves owner traversal, so the owner can create, rename, and delete entries.

### Runtime prompt

When an ordinary command is blocked by an eligible permission problem:

1. Display the path, current mode, required mode, and reason.
2. Ask before applying the permission repair.
3. Apply the mode change directly after consent.
4. Retry the blocked operation once.

Declining, using `--no-input`, lacking a TTY, wrong ownership, a symlink, or an unsafe path causes no mutation. The error includes a shell-quoted manual `chmod` command when manual repair is valid.

### Doctor workflow

- `sshx doctor` remains report-only.
- `sshx doctor --fix-permissions` gathers all eligible repairs, displays one plan, and asks for one confirmation.
- Non-interactive execution never applies repairs.
- After confirmation, doctor attempts every planned path even if one repair fails.
- The result reports `fixed`, `skipped`, and `failed` per path.
- Exit status is nonzero when any repair failed or any unsafe permission finding remains.
- Ownership mismatch, symlink, wrong path type, and shared-path findings remain diagnostics; doctor never attempts `chown` or path replacement.

### Prevention

All sshx writers create new private files as `0600` and new private directories as `0700`. In particular, saving `~/.config/sshx/config.json` must not depend on process `umask` and then require a later repair.

## Live fuzzy HostEntry picker

### Scope

Use the same picker for every interactive HostEntry selection, including:

- `connect`
- direct and paired tunnel starts
- `host update`, `host rename`, and `host delete`
- both HostEntry selections during `pair setup` or `pair create`
- any other flow that currently falls back to interactive HostEntry selection

Only `connect` opens the host action menu after selection. Other commands continue their requested operation.

Explicit selectors remain exact and deterministic. There is no automatic fuzzy fallback for `sshx connect prod` or any other supplied selector.

### Rows and searchable fields

- Render one row per alias, not one row per HostEntry.
- Selecting a secondary alias preserves that alias for the resulting OpenSSH invocation or copied command.
- Each row shows alias, destination, project and scope when present, and source path with line number.
- Duplicate aliases remain separate rows and show enough identity data to distinguish their HostEntries.
- Search alias, destination, and project. Alias matches have highest relevance.

### Ordering

With an empty query, sort aliases case-insensitively and use source path plus line number as deterministic tie-breakers.

With a query, rank in this order:

1. Exact alias match
2. Alias prefix match
3. Remaining fuzzy score
4. Alias, source path, and line number as deterministic tie-breakers

No recent-host state is stored.

### Interaction

- Query edits update the visible result set immediately.
- Arrow keys move selection.
- Enter selects the highlighted row.
- Escape or Ctrl-C performs no side effect, writes `Cancelled.` to stderr, and exits `130`.
- The picker is built into `sshx`; it does not invoke or require `fzf`.
- If an interactive selection is required without a usable TTY, return an actionable selector-required error.

## ProxyCommand

### Supported routes

Support `ProxyCommand` for:

- Direct interactive connections
- Direct standalone tunnels

Pair routes continue to reject `ProxyCommand` on either gateway or VM because Pair owns a separate transit route. The error must identify that conflict.

### Exact HostEntry boundary

Only a `ProxyCommand` directive inside the exact selected HostEntry block is supported. This feature does not add global directives, wildcard Host blocks, `Host *`, or OpenSSH inheritance semantics to discovery or runtime compilation.

### Native execution contract

- Copy the complete `ProxyCommand` directive into the private runtime config without parsing the shell command.
- Do not expand, rewrite, or allowlist OpenSSH tokens such as `%h`, `%p`, or `%r`.
- Let the OpenSSH engine validate, expand, and execute the directive exactly as it does for a native SSH config.
- Do not add an sshx confirmation prompt before executing a directive from the selected config source.
- Preserve existing sshx host-key, authentication, runtime ownership, and cleanup behavior around the OpenSSH process.

`ProxyJump` remains unsupported.

## Host actions

### Interactive flow

`sshx connect` with no selector and a usable TTY performs:

1. Live fuzzy HostEntry selection
2. Host action menu
3. The selected action

`Connect` is the default action. An explicit selector with no `--action` keeps current behavior and connects immediately.

### Direct action option

```text
sshx connect [SELECTOR] --action connect|copy-ssh|copy-sshx|copy-password
```

The option skips the action menu. It does not skip required config-root selection or stored-password confirmation.

`--format json|yaml` retains its existing inspection-only behavior and cannot be combined with `--action`.

### Availability matrix

| Selected route | Connect | Copy SSH | Copy sshx | Copy password |
| --- | --- | --- | --- | --- |
| Direct HostEntry without stored password | yes | yes | yes | no |
| Direct HostEntry with stored password | yes | yes | yes | yes |
| VM reached through a Pair | yes | no | yes | no |

Unavailable interactive actions are hidden. Requesting an unavailable action explicitly returns an actionable error and performs no fallback action.

### Copy SSH

For a direct HostEntry, copy a POSIX-shell-quoted command equivalent to:

```text
ssh -F <config-root> <selected-alias>
```

If one config root reaches the selected HostEntry, use it. If multiple config roots reach it, show their scope, project, and path and ask the user to choose. Without a TTY, multiple possible roots require explicit config-root disambiguation and return an error otherwise.

The selected alias is the exact alias row chosen in the picker or the exact alias supplied by the caller.

### Copy sshx

Copy a POSIX-shell-quoted `sshx connect` command that resolves the exact selected HostEntry. Start with its stable `--id`; include config-root or source disambiguators when needed to prevent another HostEntry from matching. This action is available for both direct and Pair-routed HostEntries.

### Copy password

- Only a non-empty password already stored in the selected direct HostEntry is eligible.
- The interactive menu hides the action when no stored password exists.
- `--action copy-password` returns an error when no stored password exists.
- Even with the explicit action, display a warning and require TTY confirmation immediately before copying.
- `--no-input` or no TTY always rejects this action.
- Never print the password to stdout or stderr, place it in argv or environment variables, or build a plaintext `sshpass` command.
- Do not promise automatic clipboard clearing. State that clipboard contents and clipboard-manager history may retain the secret until the user overwrites or removes it.

### Clipboard backends

Use native clipboard commands without a shell:

- macOS: `pbcopy`
- Linux: first compatible available backend among `wl-copy`, `xclip`, and `xsel`

Pass clipboard content through stdin, never as a command-line argument.

If no backend is available:

- A non-secret copied command is written to stdout so the caller can still use or redirect it.
- A password is never written to stdout; return an actionable error naming supported clipboard tools.

On successful clipboard copy, write only a non-secret status message.

## Compatibility and error behavior

- Existing explicit selectors keep exact-match semantics.
- Existing non-interactive connect scripts keep immediate-connect behavior when `--action` is absent.
- Existing `connect --format json|yaml` callers remain inspection-only.
- Help exits `0`; parse and unavailable-action errors exit `2`; picker cancellation exits `130`.
- No feature creates a second source of HostEntry truth.
- No feature weakens anonymous-FD password delivery used by actual connections.

## Acceptance scenarios

1. Every root, group, leaf, and accepted alias listed above returns path-specific help through `--help`, `-h`, and `sshx help <path>`.
2. A current-user-owned password-bearing config file at `0644` can be repaired to `0600` after one explicit prompt, then the blocked operation retries.
3. A wrong-owner file, symlink, or shared path is never mutated and receives actionable diagnostics.
4. `doctor --fix-permissions` continues after one chmod failure and reports every planned path.
5. Typing a fuzzy query narrows and reranks alias rows live; supplying the same text as an explicit selector never selects a fuzzy-only match.
6. Selecting a secondary alias connects or copies with that alias, not the block's first alias.
7. Escape and Ctrl-C leave no connection, mutation, tunnel, or clipboard side effect and exit `130`.
8. A direct HostEntry containing `ProxyCommand cloudflared access ssh --hostname %h` works for a direct interactive connection and a direct standalone tunnel.
9. The same directive on a Pair gateway or VM remains rejected with a Pair-route conflict.
10. Direct Copy SSH uses the selected config root and alias with correct shell quoting.
11. Pair Copy sshx produces an exact `sshx connect` command and never claims a native SSH command can reproduce the Pair transit route.
12. Copy password requires a stored password, warning, TTY confirmation, and a working clipboard backend; the secret never appears in process arguments, environment variables, stdout, or stderr.
13. A missing Linux clipboard backend prints non-secret commands to stdout but never prints a password.
14. `--action` combined with JSON or YAML output fails clearly without performing either action.

## Domain documentation decision

No glossary entry is required. Permission repair, host action, fuzzy picker, and clipboard are general UX or implementation terms rather than concepts specific to this domain.

No ADR is required. These UX choices are localized and reversible; none meets all three ADR criteria of being hard to reverse, surprising without context, and selected through a durable architectural trade-off. Existing ADR 0001 remains authoritative for the OpenSSH engine, filesystem source of truth, and Pair ownership boundaries.
