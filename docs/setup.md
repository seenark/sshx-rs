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

In Hosts, press `Ctrl+U` to update the selected HostEntry or `Ctrl+R` to rename only its selected alias. `sshx host update HOST` and `sshx host rename HOST` continue in the editor when no change is supplied; `sshx tui host update HOST --hostname DESTINATION` and `sshx tui host rename HOST --alias ALIAS` open it even with complete fields. Fields show `keep`, `replace`, or `clear`; a password marked `keep` is never revealed, typing replaces it, and `Ctrl+X` clears it. Interactive review always requires `Enter`, including with `--yes`; `--no-input`, password stdin, and JSON/YAML output never open the editor.

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

## Pair setup and paired shell

A **Pair** links exact gateway and VM HostEntries. The gateway OpenSSH master opens a temporary loopback transit forward. A separate VM OpenSSH master uses that port and its own host-key identity.

Create a Pair when one route is unambiguous:

```sh
sshx pair setup gateway-alias vm-alias
```

Provide the transit destination when inference is not unique:

```sh
sshx pair setup gateway-alias vm-alias \
  --transit-host vm.internal \
  --transit-port 22
```

Inspect Pair records and validation diagnostics:

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

Normal shell connections do not ask about service forwarding. Request a service explicitly:

```sh
sshx connect vm-alias --forward 5432=15432 --forward 6379
```

`--forward REMOTE[=LOCAL]` keeps listeners on `127.0.0.1` by default. `--bind` selects declared `##PORT` services. A requested local port conflict returns a stage-specific error; sshx does not close an unrelated process or silently choose another port.

## Standalone tunnel lifecycle

A **standalone tunnel** owns its detached OpenSSH master, control socket, runtime config, listeners, and Pair dependencies. Ownership is recorded in the locked registry; a PID alone never proves ownership.

Start a direct tunnel and return to the shell:

```sh
sshx tunnel direct start db-prod \
  -L 127.0.0.1:15432:db.internal:5432 \
  --no-input
```

Manage it from another process:

```sh
sshx tunnel direct list
sshx tunnel direct status <tunnel-id>
sshx tunnel direct restart <tunnel-id>
sshx tunnel direct stop <tunnel-id>
```

Start a Pair-owned standalone tunnel:

```sh
sshx tunnel paired start vm-alias --forward 5432=15432 --no-input
sshx tunnel paired list
```

Only an exact active request can reuse its persisted tunnel ID. Different source identities, routes, or forwards do not share a tunnel. If a control socket is dead, status reports stale/down evidence and does not reconnect automatically or kill an unproved process.

## Local verification boundary

Run fixture/local validation without making a claim about a real server:

```sh
cargo test --all-targets --all-features
./scripts/test-release-package.sh
```

These checks use temporary config trees, local process fixtures, and CI runners. They prove packaging, CLI behavior, and local OpenSSH interactions only. Real user-server authentication, host-key enrollment, host-key rotation, and standalone tunnel lifetime require separate manual validation in an authorized environment and are not implied by this setup guide.
