# Set up sshx

`sshx` uses the system **OpenSSH engine**. Install the OpenSSH client before using a config root. Key-only workflows do not need `sshpass`.

## Install prerequisites

### macOS

Use Homebrew's maintained formula. Do not install an old tap:

```sh
brew install openssh sshpass
```

`sshpass` is needed only when a selected HostEntry uses password authentication or a command supplies a password descriptor. Prefer an SSH key or agent when available.

### Ubuntu Linux

These are the commands exercised by the Linux CI job before its behavior suite:

```sh
sudo apt-get update
sudo apt-get install --no-install-recommends -y openssh-client sshpass
```

This guide makes no claim about other Linux distributions. Adapt package names only after verifying them on that distribution.

## Install sshx with mise

Release assets are published by version tag from `github:seenark/sshx-rs`. In a new shell:

```sh
mise use -g github:seenark/sshx-rs
sshx --version
```

To pin a version:

```sh
mise use -g github:seenark/sshx-rs@0.2.1
```

By default, mise's unpinned latest resolution excludes releases younger than 24 hours. The unpinned command may select an older eligible stable release or find none immediately after publication. An explicit version pin bypasses this age filter and can install the newly published version immediately, without changing the default policy. See mise's [minimum release age security policy](https://mise.jdx.dev/security.html#minimum-release-age) and [setting reference](https://mise.jdx.dev/configuration/settings.html#minimum_release_age).

After installation, `mise which sshx` reports the binary path. This backend installs it under `$MISE_DATA_DIR/installs/github-seenark-sshx-rs/<version>/sshx`; mise downloads and extracts the matching GitHub Release archive there. Copying a build into that directory is not a substitute for installation or release selection.

Supported release targets are:

- `aarch64-apple-darwin` for Apple Silicon M1 and later;
- `aarch64-unknown-linux-gnu` for GNU/Linux arm64;
- `x86_64-unknown-linux-gnu` for GNU/Linux x86_64.

GNU/Linux uses Ubuntu 22.04 as its compatibility baseline. Mac Intel, Windows, and Alpine/musl are not supported. Apple Silicon support does not promise compatibility with every historical macOS version.

Each release contains exactly three `.tar.gz` archives and three companion `.sha256` files. GitHub Release assets are the distribution source; mise downloads the matching target asset, extracts executable `sshx` from archive root, and creates local installation state. Copying a binary into mise's cache or installation directories is not equivalent to a GitHub backend installation. Archives do not bundle the system OpenSSH engine or optional `sshpass`; the prerequisites above still apply.

## Register config roots

The **filesystem source of truth** is the SSH config on disk. `sshx` does not import hosts into a second database. It traverses `Include` directives from each **config root** whenever it runs.

Default personal root is `~/.ssh/config`. This environment also supports the existing work root `~/.private-key/private-key/config`; setup must not invent `~/.private-key/config`.

Run `sshx` with no arguments to open Hosts. Run `sshx --config PATH` to open Hosts against an explicit config root; this also requires usable terminals.

The Hosts TUI has no copy keys yet; run `sshx connect HOST --action copy-password` to copy a stored password. It requires a direct HostEntry with a non-empty stored password and a supported clipboard backend. The copy needs no confirmation unless the `password_confirm` setting is on; then answer `y` to `Copy stored password now? [y/N]:`. The clipboard-history warning appears on the first copy only. Copying, declining, or an unavailable action returns to Hosts with the same search and selection; it never starts SSH or displays the password. After `clipboard_clear` seconds (default 30), a detached helper clears the clipboard if it still holds the copied password. Clipboard backends without a readback tool are never cleared.

Let setup discover existing roots:

```sh
sshx setup
```

Register explicit roots:

```sh
sshx setup \
  --personal ~/.ssh/config \
  --work ~/.private-key/private-key/config
```

Register another scope or project without moving files:

```sh
sshx setup --project payments --root work=~/work/ssh/config
```

Run `sshx setup` for Setup or `sshx doctor` for Doctor. Setup lists registered roots and lets you edit scope (usually personal/work), optional project, and an existing SSH config path. Press **Ctrl+S** to register the displayed root; opening or editing Setup never registers it. Missing paths, directories, and symlinked root files do not register. **Esc** cancels without changing SSH config or root settings.

Partial `sshx setup --scope work --project payments` continues in Setup when no work root is discovered on a usable TTY, with scope and project prefilled and the path focused. Complete root options retain direct CLI behavior, including `--config PATH` with inferred scope. Use `sshx tui setup --config PATH --scope work --project payments` to edit even a complete request before registering. Successful registration returns to Hosts and rediscovers HostEntries from SSH config files. Without usable terminals, with `--no-input`, or with machine output, missing roots produce an actionable CLI error instead of opening Setup.

Check roots, Include diagnostics, permissions, OpenSSH, host-key paths, ports, and runtime state:

```sh
sshx doctor
sshx doctor --format json
```

`doctor` groups findings by severity and stage, with path, evidence, and next action. It reports only by default. Run `sshx tui doctor`, then **Repair eligible permissions** to review one explicit plan. The CLI equivalent is `sshx doctor --fix-permissions`. Both require one confirmation before chmod. Wrong-owner, symlink, shared, and otherwise ineligible paths stay unchanged. Doctor never enrolls host keys, rewrites config, adopts processes, or stops tunnels.

## The Hosts TUI

`sshx` with usable terminals opens Hosts, a modal finder in the style of Helix and Neovim. Typing searches (Insert input mode, the default); `Esc` switches to Normal input mode and never quits. `i`, `a`, or `/` return to Insert. `q` or `:q` quits from Normal input mode; `Ctrl+C` quits from any input mode with status `130`.

- Empty search shows a RECENT group (from per-HostEntry usage state), then every HostEntry grouped by config root. A typed query sorts by fuzzy rank, then use frequency, then alias. Duplicate aliases keep their source and ID labels. `⇄N` counts declared services.
- A preview pane shows the route (direct, or `gateway → VM` for a Pair), HostName, declared services, and last use. It collapses below 70 columns.
- `Enter` opens Session mode immediately with no service forwards and no prompts. After the session ends, sshx exits with ssh's exit code.
- Movement: `j`/`k` or arrows, counts such as `5j`, `gg`, `ge`, `^d`, `^u`. `g r`, `g p`, `g w` jump to the recent, personal, and work groups; `g t` opens the tunnel list. `space f p|w|r` filter to the personal root, work root, or recent HostEntries; repeating a filter clears it.
- A which-key box opens on every prefix key (`space`, `g`, `y`, `d`, `c`, `Z`); `Esc` or `Backspace` goes up one level. `?` (Normal input mode, or Insert with an empty search) and `F1` open help, grouped by section and filterable.
- `:` opens a command line with `Tab` completion: `:q`, `:help`, `:ssh HOST [REMOTE[=LOCAL]...]`, `:set OPTION[=VALUE]`, `:root add|remove PATH`, `:tunnel list`. `:set` persists `startmode` (`insert`|`normal`), `autoinfo` (`false` hides the which-key box), `bind` (loopback only), `clipboard_clear` (`off` or seconds), and `password_confirm` in the sshx settings file.
- The status line shows input mode, config-root filter, position, running standalone tunnels, toast, and pending keys.
- With no registered config roots, a first-run screen lists detected SSH config files (`~/.ssh/config`, `~/.ssh/config.d/*`). Tick with `x`, label with `p` (personal) or `w` (work), and press `Enter` to register them. `s` skips setup, `q` or `:q` quits, `?` lists these keys, and `Esc` never quits.

## List and select HostEntries

A **HostEntry** is an exact `Host` block identified by source file and Host line. An alias is not a unique identity; duplicate aliases remain separate entries.

In the finder, `d d` shows the exact block (passwords redacted) and asks `y`/`n`; `u` undoes it afterwards. Pair members and HostEntries used by a running standalone tunnel are refused. Incomplete `sshx host delete` opens the finder where `Enter` opens that confirmation, and `sshx tui host delete HOST` opens it for an exact HostEntry; both return to the finder afterwards. With `--preview`, `y` prints the review without deleting anything. Fully specified CLI deletion retains its existing consent behavior.

```sh
sshx host list
sshx host list --scope work --project payments
sshx host show db-prod
sshx host show <entry-id>
```

Use an ID, or combine an alias with its source file and Host line, when automation needs deterministic selection:

```sh
sshx connect --id <entry-id> --no-input
sshx connect db-prod --source ~/.ssh/hosts.conf --line 42 --no-input
```

`sshx tui connect HOST`, `sshx tui tunnel start HOST`, and incomplete `connect` or `tunnel start` commands continue the requested operation on usable stdin and stderr terminals. Without a selector they open the finder (Enter starts a Session; for `tunnel start` Enter opens the ports screen in Tunnel mode); with an exact alias, ID, or source/line identity they open the ports screen for that HostEntry. Unknown, ambiguous, or conflicting explicit selectors and malformed forwarding options fail before the UI opens. `-L`, `-R` and `-D` (direct routes only; a Pair route rejects them with `PAIR_FORWARD_INVALID`) are listed on the ports screen as fixed rows such as `-L 127.0.0.1:15432:db.internal:5432 (from command line)` and open with the Session or Tunnel together with the checked service forwards. `--password-fd`, `--gateway-password-fd` and `--vm-password-fd` reach the Session or Tunnel exactly as without the TUI, and a retry after an authentication failure reuses the retained descriptor value. `--bind` opens the ports screen for the chosen HostEntry. An interrupted tunnel start exits `130`.

Bare `sshx tui --id ID` or `sshx tui --source FILE --line N` prefills the exact Hosts row without locking subsequent browsing to that identity. Source selectors resolve the physical file identity, including normalized paths, and still require an exact Host line.

On the ports screen, `--forward` values and the forwards selected last time are checked; `x` toggles a row, `o` adds `REMOTE[=LOCAL]`, and `Enter` connects (or starts the tunnel in Tunnel mode). Cancelling with `Ctrl+C` exits with status `130` without starting a connection, changing files, or copying anything. `--no-input`, JSON/YAML, password stdin, and unusable terminals never open a continuation. `sshx host show --id ID` inspects the exact entry; `sshx host show` without a selector opens the finder and `Enter` prints the chosen entry.

If a selected source changes or disappears, the continuation shows the error without starting the action. Failed rediscovery removes stale choices but leaves cancellation available with the same pending status.

## Update, rename, and delete HostEntries

```sh
sshx host update db-prod --hostname db.internal
sshx host rename db-prod --alias database
sshx host delete database
```

Duplicate aliases need `--id`, or alias plus `--source` and `--line`. Complete CLI mutations keep preview and consent behavior. `--preview` writes nothing; `--yes` confirms explicitly. With `--preview` the TUI form (or delete confirmation) says `preview only`, and `:w` (or `y`) prints the same review as the CLI and exits without writing.

`sshx host update HOST` and `sshx host rename HOST` open the host form when no change is supplied; `sshx tui host update HOST --hostname DESTINATION` and `sshx tui host rename HOST --alias ALIAS` open it with the supplied values filled in. The form shows a live preview of the exact block that will be written; a stored password is never revealed, typing replaces it, and backspace clears it. Save with `:w`, `:wq` or `ZZ`; `:q` cancels. Without a selector the finder opens and `Enter` opens the form. `--no-input`, password stdin, and JSON/YAML output never open the form.





If Pair metadata blocks deletion, the confirmation refuses it and names the Pair.




## Direct shell

Open one HostEntry through its normal OpenSSH settings:

```sh
sshx connect db-prod
```

With password input supplied by a caller, pass an inherited file descriptor. Never put a password in argv or environment:

```sh
sshx connect db-prod --password-fd 3 --no-input 3<"$HOME/.ssh/password"
```

The selected HostEntry's `IdentityFile`, agent options, and supported OpenSSH settings stay in effect. Host-key enrollment is separate from authentication. On first use, interactive mode shows the OpenSSH fingerprint and asks for explicit confirmation; non-interactive mode returns `HOST_KEY_TRUST_REQUIRED`.

When the shell ends, `sshx connect` exits with the shell's exit code, or 128+N when ssh is killed by signal N. A remote command's non-zero exit is passed through without an error message.

With `sshx connect` and no selector, an interactive terminal opens the finder; `Enter` starts a Session with no service forwards and no port-binding step, and `Tab` opens the ports screen. Use `sshx tui connect HOST` to open the ports screen for an exact HostEntry; complete `sshx connect HOST` remains direct. Copy actions remain available with explicit `--action copy-ssh`, `--action copy-sshx`, or `--action copy-password`; without a selector `Enter` in the finder performs the action. Copy SSH applies to direct HostEntries, and Copy sshx supports direct and Pair routes. Copy password requires an eligible stored direct password and a supported clipboard backend, confirms only when `password_confirm` is on, and clears the clipboard after `clipboard_clear` seconds. The secret never appears in terminal output, command arguments, or environment variables.

The finder preview shows the HostEntry ID, source file and Host line. Duplicate aliases show source and ID in their rows. Selecting a secondary alias preserves it for copy actions.

## Create HostEntries

Run `sshx host create` on a terminal to open the new-host form, or `sshx host create --alias prod` to open it with `prod` prefilled (`--hostname`, `--user`, `--port`, `--file` and a single matching `--scope`/`--project` root prefill too). Only alias and host are required. The `file` field picks a registered config root or included file. Password input stays masked and the destination file must deny group and other access before storing a password; new files use mode `0600`.

The form previews the exact block while you type. Save with `:w`, `:wq` or `ZZ`; `:q` cancels without changing SSH config. With `--preview`, `:w` prints the review and exits without writing.

A scope conflicting with explicit `--config` fails before opening the form. After saving, the form returns to the finder.
Complete CLI commands run directly:

```sh
sshx host create --scope personal --file ~/.ssh/config --alias prod --hostname prod.example --yes
```

## Pair setup and paired shell

A **Pair** links exact gateway and VM HostEntries. The gateway OpenSSH master opens a temporary loopback transit forward. A separate VM OpenSSH master uses that port and its own host-key identity.

Run `sshx tui pair list` or `sshx tui pair validate` to open Pairs. Use Up/Down to select a Pair or a diagnostic group. The detail pane shows exact gateway and VM aliases, IDs, source paths and Host lines, approved transit, and route validity. Diagnostic groups show evidence and guidance; use Page Up/Page Down to scroll details. Press `V` to rediscover and validate current SSH config without changing files, repairing permissions, or starting OpenSSH. Press `Esc` to return to Hosts (then `Esc`, `q` to quit), or `S` to begin Pair setup.

Press `D` on an exact Pair record to review deletion. Review shows both aliases, immutable IDs, source paths and Host lines, stored transit, and a redacted diff. `Enter` deletes the relationship; `Esc` or `Ctrl+C` cancels and returns to Pairs. Both HostEntries, IDs, passwords, comments, and OpenSSH directives—including existing forwards—remain unchanged. Pairs refreshes after deletion, and the unlinked HostEntries can be paired again. Diagnostic groups and empty rows cannot be deleted.

Active, starting, or stopping managed use of either endpoint blocks Pair deletion. Unsafe or invalid registries, conflicting relationship metadata, and pending recovery journals also block it. Changed route directives alone do not prevent unlinking an unambiguous reciprocal relationship. Deletion rechecks current discovery, exact sources, journals, and managed use after review, then applies only the reviewed plan; changed source bytes are rejected without writes. It never starts or stops OpenSSH.

Create a Pair when one route is unambiguous:

```sh
sshx pair setup gateway-alias vm-alias
```

Provide the transit destination when inference finds multiple candidates. An explicit destination must match exactly one gateway `LocalForward` remote destination.

For a VM whose `HostName` is loopback or `localhost`, a gateway listener matching the VM address and `Port` supplies the remote transit destination. The remote port may differ from the VM `Port`: `LocalForward 2222 vm.internal:22` matches a VM at `127.0.0.1:2222`, with transit `vm.internal:22`. If no local listener matches, the transit destination port must match the VM `Port`. Pair setup preserves the original directives and VM port.

```sh
sshx pair setup gateway-alias vm-alias \
  --transit-host vm.internal \
  --transit-port 22
```

An incomplete command such as `sshx pair setup gateway-alias`, or a route with unresolved transit, opens one Pair workspace when stdin and stderr are usable terminals. Run `sshx tui pair setup gateway-alias vm-alias` to edit even a complete operation. Supplied exact selections remain visible and editable; explicit unknown or ambiguous selectors fail before the workspace opens. Machine output, piped input, and `--no-input` never open it.

Pair setup is one form with **Gateway**, **VM**, and **Transit** sections. At 80 columns and 20 rows or larger, the form and active editor appear side by side; smaller terminals stack them and keep the focused field visible. Committed gateway and VM aliases remain separate from the active `Search:` query. Type to search eligible aliases and press `Enter` to choose an exact HostEntry. A search with no matches does not replace the committed selection. Candidate context identifies the exact source and ID; use Page Up/Page Down to scroll context on small terminals.

Use `Tab` or `Shift+Tab` to move between gateway, VM, transit host, and transit port. Gateway selection excludes the same HostEntry, already-paired entries, and unsafe proxy routes. Missing, ambiguous, or incompatible transit does not hide an otherwise eligible VM; setup validates transit before review. If a changed gateway makes the selected VM structurally ineligible, setup retains the VM and warns until you replace it. The Transit section keeps host and port together and shows automatic, ambiguous, or missing transit candidates. Press `Ctrl+N` to choose and cycle candidates explicitly, or edit host and port inline. Ambiguous transit is never silently selected.

Press `Ctrl+S` to replace the form with a full-width review of both exact sources, transit, and metadata changes; use Page Up/Page Down to scroll. Only `Enter` during review applies the existing atomic Pair mutation. `Esc` returns to the unchanged draft, then cancels; `Ctrl+C` cancels immediately. Editing invalidates review and requires another `Ctrl+S`. Validation errors retain values. Changed sources require reopening setup, and cancellation or failed validation writes no relationship. A successful result leaves the workspace open. `--yes` skips only the complete CLI consent prompt, not TUI review. At 32×10, focused editing and review/cancel hints remain visible; below that size, cancellation remains available but not all context fits.

Use `--preview` or `--dry-run` to render Pair changes without writing config. In the workspace, a successful review completes preview without allowing Apply. Complete CLI commands retain direct preview and consent behavior.

When a recovery journal is pending, Pairs blocks both setup and deletion. Press `R` to inspect affected files and give separate explicit recovery consent; recovery returns to the refreshed Pairs list without starting setup or deletion. Pair setup from the CLI likewise lists affected files and asks for explicit recovery consent before opening setup. Declining changes nothing. Confirming restores the interrupted transaction's saved content, then rediscovers HostEntries. `--yes` does not bypass recovery review; non-interactive or machine-output setup remains blocked until interactive recovery completes.

Inspect Pair records and validation diagnostics from the CLI:

```sh
sshx pair list
sshx pair validate
```

Open a paired VM shell in the current terminal:

```sh
sshx connect vm-alias
```

Gateway and VM passwords can be supplied separately with `--gateway-password-fd` and `--vm-password-fd`. A paired route never replaces the source `LocalForward`, `HostName`, or `Port` in the user's config.

## Service forwards

The ports screen selects service forwards for TUI connections. CLI shell connections do not prompt. Repeat `--forward REMOTE[=LOCAL]` to request several declared services in one operation. The left port is the service port on the server; the right port is the local listener port, which binds to `127.0.0.1` by default:

```sh
sshx connect vm-alias --forward 5432=5432 --forward 6379=6378 --forward 3001=3001
sshx tunnel direct start db-prod --forward 5432=5432 --forward 6379=6378 --forward 3001=3001 --no-input
```

`sshx tui connect HOST` and `sshx tui tunnel start HOST` open the ports screen for a direct HostEntry or Pair VM. Rows are declared services plus the forwards selected last time; `x` toggles, `o` adds `REMOTE[=LOCAL]`, `s` declares a row, `dd` removes one. `Enter` connects (Tunnel mode starts a standalone tunnel after resolving busy local ports). Session permits no selected rows; Tunnel requires at least one.

Direct Session and standalone Tunnel commands can mix declared services with custom `-L`, `-R`, and `-D` rows:

```sh
sshx connect db-prod --forward 5432=5432 -L 127.0.0.1:6378:cache.internal:6379 -R 127.0.0.1:15432:db.internal:5432 -D 127.0.0.1:1080 --no-input
sshx tunnel direct start db-prod --forward 5432=5432 -L 127.0.0.1:6378:cache.internal:6379 -R 127.0.0.1:15432:db.internal:5432 -D 127.0.0.1:1080 --no-input
```

`-R` listens on the server. Connections go to the destination on your side. Local preflight cannot prove the server listener is available; OpenSSH reports server bind failures. Remote listeners can expose services beyond loopback. A specific non-loopback remote bind requires `--allow-bind` in both Session and Tunnel mode; sshx warns about that exposure before launching OpenSSH. Review bind address and server policy before starting.

`-D` opens a local SOCKS proxy. Configure applications explicitly to use its bind address and port; sshx does not configure application proxy settings.

Custom listeners in direct Session and standalone Tunnel mode accept IPv4 and IPv6 loopback addresses. Specific non-loopback addresses require `--allow-bind`; wildcard binds remain rejected. Pair routes accept only declared VM services and reject custom `-L`, `-R`, or `-D` rows before startup.

Session mode closes its session-bound master and forwards when the shell exits. A session started from the finder or a TUI continuation ends sshx with ssh's exit code. A standalone Tunnel remains active after leaving the TUI; use its displayed Tunnel ID to manage it.

`--bind` selects declared `##PORT` services interactively; with `sshx tui` or a finder continuation it opens the ports screen for the chosen HostEntry. A requested local port conflict returns a row-specific error; sshx does not close an unrelated process or silently choose another port.

## Standalone tunnel lifecycle

A **standalone tunnel** owns its detached OpenSSH master, control socket, runtime config, listeners, and Pair dependencies. Ownership is recorded in the locked registry; a PID alone never proves ownership.

Open the tunnel list from the finder with `g t` in Normal input mode or `:tunnel list`; the status line shows the number of running standalone tunnels. The list shows persisted Tunnel IDs from earlier runs, including active, down, stopped, or stale registry evidence, with route, exact source, mappings, master responsiveness, and listener readiness as separate evidence. It does not report application health. `dd` stops the selected tunnel and `r` restarts it. Unproved or unavailable actions report why and never use a PID alone.

`sshx tunnel HOST` and `sshx tunnel start HOST` select the HostEntry's route automatically. Host aliases named `direct` or `paired` remain valid in the automatic form. Use `tunnel direct start HOST` or `tunnel paired start HOST` to require a route; a mismatch returns `TUNNEL_ROUTE_MISMATCH` without starting a Tunnel. Route-specific lists contain only that route's records, and status, stop, and restart reject IDs belonging to the other route before changing owned resources. All forms use the same persisted registry.

An incomplete route-specific start continues in Tunnel mode. After a failed direct startup, retrying the same unchanged HostEntry reuses a supplied password descriptor's retained value rather than reading the consumed descriptor again. Each explicit retry allows one password correction if authentication still fails, including after a previously prompted password fails. Selecting another HostEntry or changing its source discards the retained value. A route-specific retry still rejects the opposite route. Leaving the completed continuation closes sshx's piped stdout without stopping its standalone Tunnel; stop that Tunnel explicitly by ID.

Start a direct tunnel and return to the shell:

```sh
sshx tunnel direct start db-prod \
  -L 127.0.0.1:15432:db.internal:5432 \
  --no-input
```

Manage direct tunnels by persisted Tunnel ID from another process:

```sh
sshx tunnel direct list
sshx tunnel direct status <tunnel-id>
sshx tunnel direct restart <tunnel-id>
sshx tunnel direct stop <tunnel-id>
```

Start a Pair-owned standalone tunnel and manage its IDs:

```sh
sshx tunnel paired start vm-alias --forward 5432=15432 --no-input
sshx tunnel paired list
sshx tunnel paired status <tunnel-id>
sshx tunnel paired restart <tunnel-id>
sshx tunnel paired stop <tunnel-id>
```

Only an exact active request can reuse its persisted tunnel ID. Different source identities, routes, or forwards do not share a tunnel. If a control socket is dead, status reports stale/down evidence and does not reconnect automatically or kill an unproved process.

With usable interactive terminals, `sshx tunnel status`, `sshx tunnel stop` and `sshx tunnel restart` without an ID open the tunnel list (`dd` stops, `r` restarts, `Enter` opens SSH through the master). Route-specific forms filter the list. `--no-input`, machine output, and pipes stay CLI-only and require an ID.

## Local verification boundary

Run fixture/local validation without making a claim about a real server:

```sh
cargo test --all-targets --all-features
./scripts/test-release-package.sh
```

PTY integration checks run one interactive fixture at a time so concurrent fixture startup does not consume the interaction deadline. Failed checks terminate and reap their owned fixture processes; they do not stop unrelated processes or lengthen the existing deadlines.
Each fixture owns a controlling PTY with initialized window dimensions. Detached fake OpenSSH masters use separate sessions so launcher exit does not stop a standalone tunnel.
Checks assert exact HostEntry IDs and observable state rather than wrapped path strings or offscreen labels.

These checks use temporary config trees, local process fixtures, and CI runners. They prove packaging, CLI behavior, and local OpenSSH interactions only. Real user-server authentication, host-key enrollment, host-key rotation, and standalone tunnel lifetime require separate manual validation in an authorized environment and are not implied by this setup guide.
