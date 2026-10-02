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
mise use -g github:seenark/sshx-rs@0.1.0
```

The release contains one archive for each supported OS/architecture pair. mise selects the matching target asset and extracts executable `sshx` from archive root.

## Register config roots

The **filesystem source of truth** is the SSH config on disk. `sshx` does not import hosts into a second database. It traverses `Include` directives from each **config root** whenever it runs.

Default personal root is `~/.ssh/config`. This environment also supports the existing work root `~/.private-key/private-key/config`; setup must not invent `~/.private-key/config`.

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

In Hosts, press **Ctrl+S** for Setup or **Ctrl+D** for Doctor, including when no HostEntries are found. Setup lists registered roots and lets you edit scope (usually personal/work), optional project, and an existing SSH config path. Press **Ctrl+S** to register the displayed root; opening or editing Setup never registers it. Missing paths, directories, and symlinked root files do not register. **Esc** cancels without changing SSH config or root settings.

Partial `sshx setup --scope work --project payments` continues in Setup when no work root is discovered on a usable TTY, with scope and project prefilled and the path focused. Complete root options retain direct CLI behavior, including `--config PATH` with inferred scope. Use `sshx tui setup --config PATH --scope work --project payments` to edit even a complete request before registering. Successful registration returns to Hosts and rediscovers HostEntries from SSH config files. Without usable terminals, with `--no-input`, or with machine output, missing roots produce an actionable CLI error instead of opening Setup.

Check roots, Include diagnostics, permissions, OpenSSH, host-key paths, ports, and runtime state:

```sh
sshx doctor
sshx doctor --format json
```

`doctor` reports problems. It does not chmod files, rewrite config, trust host keys, adopt processes, or stop tunnels silently.

## List and select HostEntries

A **HostEntry** is an exact `Host` block identified by source file and Host line. An alias is not a unique identity; duplicate aliases remain separate entries.

To delete from Hosts, press `Ctrl+X` on the selected HostEntry. Review its exact source, Host line, byte span, redacted deletion diff, and Pair dependencies before pressing `Enter`; `Esc` cancels. Pair references and active managed use block deletion. Incomplete `sshx host delete` opens the exact picker and review, and `sshx tui host delete HOST` opens review explicitly. After explicit TUI deletion or preview, Hosts shows the result and stays open until you quit. `--preview` never writes, and TUI review still requires confirmation with `--yes`. Fully specified CLI deletion retains its existing consent behavior.

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

`sshx tui connect HOST`, `sshx tui tunnel HOST`, and incomplete `connect` or `tunnel` commands continue the requested operation on usable stdin and stderr terminals. The focused HostEntry selector preselects an exact supplied alias, ID, or source/line identity. Search and arrow keys can choose another entry within the requested scope/project; identity selectors do not permanently filter the list. Unknown, ambiguous, or conflicting explicit selectors and malformed forwarding options fail before the UI opens.

Bare `sshx tui --id ID` or `sshx tui --source FILE --line N` prefills the exact Hosts row without locking subsequent browsing to that identity. Source selectors resolve the physical file identity, including normalized paths, and still require an exact Host line.

After selection, supplied forwarding values appear in the Session or Tunnel workspace and remain editable before review. `Esc` returns to the focused selector; cancelling before a successful operation exits with status `130` without starting a connection, changing files, or copying anything. Completed operations stay interactive. Password descriptors remain bound to the original exact HostEntry and source snapshot and are not replayed after a HostEntry change. `--no-input`, JSON/YAML, password stdin, and unusable terminals never open a continuation. `sshx host show --id ID` inspects the exact entry; `sshx tui host show --id ID` prefills interactive inspection.

If a selected source changes or disappears, the continuation shows the error without starting the action. Failed rediscovery removes stale choices but leaves cancellation available with the same pending status.

In Hosts, press `Ctrl+U` to update the selected HostEntry or `Ctrl+R` to rename only its selected alias. `sshx host update HOST` and `sshx host rename HOST` continue in the editor when no change is supplied; `sshx tui host update HOST --hostname DESTINATION` and `sshx tui host rename HOST --alias ALIAS` open it even with complete fields. Fields show `keep`, `replace`, or `clear`; a password marked `keep` is never revealed, typing replaces it, and `Ctrl+X` clears it. Interactive review always requires `Enter`, including with `--yes`; `--no-input`, password stdin, and JSON/YAML output never open the editor.

On compact terminals, the update and rename editors keep review and cancellation controls visible. Move between update fields with `Tab` or the arrow keys; the field list scrolls to keep the selected field visible.

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

## Create HostEntries

From Hosts, press `Ctrl+N` to open the editable HostEntry form. Run `sshx host create --alias prod` to open the same form with `prod` prefilled. The form selects an existing config root, destination file, alias, host destination, and optional fields. Password input stays masked. The destination file must deny group and other access before storing a password; new files use mode `0600`.

Press `Ctrl+S` to review the filesystem change in the same workspace. Press `Enter` to apply with explicit consent, or `E`/`Esc` to return to the form. Validation errors keep entered values and focus the affected field. Short terminals scroll with the selected field. Press `Esc` in the form or `Ctrl+C` to cancel without changing SSH config. With `--preview`, `Enter` finishes review without applying.

Use left/right arrows on the config-root field to select an existing root and prefill its scope, project, and folder. Supplied values remain editable. A scope conflicting with explicit `--config` fails before opening the form; incomplete creates without an explicit config can correct unmatched scope or project in the workspace. Creating from Hosts returns to Hosts with a completion result.
Complete CLI commands run directly:

```sh
sshx host create --scope personal --file ~/.ssh/config --alias prod --hostname prod.example --yes
```

## Pair setup and paired shell

A **Pair** links exact gateway and VM HostEntries. The gateway OpenSSH master opens a temporary loopback transit forward. A separate VM OpenSSH master uses that port and its own host-key identity.

In the Hosts TUI, press `Ctrl+P` to open Pairs, or run `sshx tui pair list` or `sshx tui pair validate`. Use Up/Down to select a Pair or a diagnostic group. The detail pane shows exact gateway and VM aliases, IDs, source paths and Host lines, approved transit, and route validity. Diagnostic groups show evidence and guidance; use Page Up/Page Down to scroll details. Press `V` to rediscover and validate current SSH config without changing files, repairing permissions, or starting OpenSSH. Press `Esc` to return to Hosts, or `S` to begin Pair setup.

Create a Pair when one route is unambiguous:

```sh
sshx pair setup gateway-alias vm-alias
```

Provide the transit destination when inference finds multiple candidates. Explicit destination must match exactly one gateway `LocalForward`, and its port must match the VM `Port`:

```sh
sshx pair setup gateway-alias vm-alias \
  --transit-host vm.internal \
  --transit-port 22
```

An incomplete command such as `sshx pair setup gateway-alias`, or a route with unresolved transit, opens one Pair workspace when stdin and stderr are usable terminals. Run `sshx tui pair setup gateway-alias vm-alias` to edit even a complete operation. Supplied exact selections remain visible and editable; explicit unknown or ambiguous selectors fail before the workspace opens. Machine output, piped input, and `--no-input` never open it.

Use `Tab` or `Shift+Tab` to move between gateway, VM, transit host, and transit port. Type to search alias rows and press `Enter` to choose an exact entry. Rows and details identify source files and Host lines, including duplicate and secondary aliases. Gateway selection excludes the same HostEntry, already-paired entries, unsafe proxy routes, and VMs without compatible transit. Press `Ctrl+N` to choose and cycle transit candidates explicitly, or edit host and port inline. Ambiguous transit is never silently selected.

Press `Ctrl+S` to review both exact sources, transit, and metadata changes in the same workspace; use Page Up/Page Down to scroll. Only `Enter` during review applies the existing atomic Pair mutation. `Esc` returns to editing, then cancels; `Ctrl+C` cancels immediately. Validation errors retain values. Changed sources require reopening setup, and cancellation or failed validation writes no relationship. A successful result leaves the workspace open. `--yes` skips only the complete CLI consent prompt, not TUI review.

Use `--preview` or `--dry-run` to render Pair changes without writing config. In the workspace, a successful review completes preview without allowing Apply. Complete CLI commands retain direct preview and consent behavior.

When a recovery journal is pending, Pair setup lists the affected files and asks separately for explicit recovery consent before opening setup. Declining changes nothing. Confirming restores the interrupted transaction's saved content, then rediscovers HostEntries. `--yes` does not bypass recovery review; non-interactive or machine-output setup remains blocked until interactive recovery completes.

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

Interactive TUI connections open a workspace for selecting declared services. CLI shell connections do not prompt. Repeat `--forward REMOTE[=LOCAL]` to request several declared services in one operation. The left port is the service port on the server; the right port is the local listener port, which binds to `127.0.0.1` by default:

```sh
sshx connect vm-alias --forward 5432=5432 --forward 6379=6378 --forward 3001=3001
sshx tunnel direct start db-prod --forward 5432=5432 --forward 6379=6378 --forward 3001=3001 --no-input
```

From Hosts, press `Enter` on either a direct HostEntry or a Pair VM to open the same connection workspace. Select rows with `Space`, edit local ports with `E`, and switch Session/Tunnel mode with `M`. Review shows exact source identity and, for a Pair, both gateway and VM identities. `Enter` opens review; a second `Enter` starts the request. Session permits no selected services; Tunnel requires at least one. A failed start keeps edited rows available for correction, and `Esc` returns to Hosts without starting another request.

`-R` listens on the server. Connections go to the destination on your side. Local preflight cannot prove the server listener is available; OpenSSH reports server bind failures. Remote listeners can expose services beyond loopback. A specific non-loopback remote bind requires `--allow-bind` in both Session and Tunnel mode; sshx warns about that exposure before launching OpenSSH. Review bind address and server policy before starting.

`--forward REMOTE[=LOCAL]` keeps listeners on `127.0.0.1` by default. `--bind` selects declared `##PORT` services. A requested local port conflict returns a stage-specific error; sshx does not close an unrelated process or silently choose another port.

## Standalone tunnel lifecycle

A **standalone tunnel** owns its detached OpenSSH master, control socket, runtime config, listeners, and Pair dependencies. Ownership is recorded in the locked registry; a PID alone never proves ownership.

`sshx tunnel HOST` and `sshx tunnel start HOST` select the HostEntry's route automatically. Host aliases named `direct` or `paired` remain valid in the automatic form. Use `tunnel direct start HOST` or `tunnel paired start HOST` to require a route; a mismatch returns `TUNNEL_ROUTE_MISMATCH` without starting a Tunnel. Route-specific lists contain only that route's records, and status, stop, and restart reject IDs belonging to the other route before changing owned resources. All forms use the same persisted registry.

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

With usable interactive terminals, `sshx tunnel status` and `sshx tunnel stop` open the Tunnels workspace for exact ID selection. Route-specific forms also work. `--no-input`, machine output, and pipes stay CLI-only and require an ID.

## Local verification boundary

Run fixture/local validation without making a claim about a real server:

```sh
cargo test --all-targets --all-features
./scripts/test-release-package.sh
```

These checks use temporary config trees, local process fixtures, and CI runners. They prove packaging, CLI behavior, and local OpenSSH interactions only. Real user-server authentication, host-key enrollment, host-key rotation, and standalone tunnel lifetime require separate manual validation in an authorized environment and are not implied by this setup guide.
