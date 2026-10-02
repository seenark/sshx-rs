# Troubleshoot sshx

Start with a machine-readable local report:

```sh
sshx doctor --format json
```

Human output names a stage and evidence. A report can prove local configuration or listener state without proving remote application health.

## Config roots and Include files

**Symptoms:** `config_root_missing`, `config_root_unreadable`, missing hosts, duplicate-looking entries, or an Include diagnostic.

1. Check the configured roots:

   ```sh
   sshx setup
   sshx doctor
   ```

2. Confirm the personal root is `~/.ssh/config` and the work root is the real existing path, commonly `~/.private-key/private-key/config` in this environment.
3. Ensure each root is a regular readable file. `sshx` rejects symlink roots and does not follow an unsafe root.
4. Resolve Include paths relative to the file where OpenSSH finds them. Preserve lexical wildcard ordering. Fix cycles or broken references in the source files.
5. Run `sshx host list --format json` again. Discovery reads the filesystem source of truth on every command; it does not use a cached host database.

`HostEntry` identity includes source path and Host line. If aliases repeat, select by ID or by alias plus `--source` and `--line`.

## Pair setup and route failures

**Symptoms:** `PAIR_INVALID`, `PAIR_REQUIRED`, `PAIR_TRANSIT`, `gateway trust`, `gateway auth`, `transit bind`, `VM trust`, or `VM auth` stage errors.

If Pair setup reports a pending recovery journal, run `sshx pair setup` in an interactive terminal to review affected paths and explicitly confirm restoring saved pre-mutation content. `--yes` does not bypass recovery consent; `--no-input`, piped input, and machine output remain blocked. Setup rediscovers HostEntries after recovery.

1. Validate exact records:

   ```sh
   sshx pair list --format json
   sshx pair validate --format json
   ```

2. Ensure the Pair names the intended gateway and VM HostEntries, not aliases that happen to match another source.
3. If transit inference has multiple candidates, select an exact `--transit-host` and `--transit-port` from the gateway's `LocalForward` directives. Transit port must match VM `Port`.
4. Confirm the gateway can reach the transit destination and that the requested temporary loopback port is free.
5. Treat gateway and VM authentication as separate checks. Supply separate password file descriptors when needed; do not retry a rejected password automatically.
6. A paired shell requires the gateway master first, then VM trust and authentication. A local listener or gateway master response is not end-to-end VM success.

Do not replace a Pair route with `ProxyJump`. The approved route uses separate OpenSSH masters so gateway and VM credentials and host-key identities stay separate.

## Direct and paired shells

**Symptoms:** `HOST_REQUIRED`, `HOST_NOT_FOUND`, `HOST_AMBIGUOUS`, or a shell opens for the wrong copied alias.

- In non-interactive mode, provide `--id` or a complete alias/source/line selector. `--no-input` never opens a picker or prompt.
- For a direct shell, use `sshx connect <alias>` only when that alias is unique in the selected roots.
- For a paired shell, select the VM HostEntry. `sshx` starts the Pair route and opens the VM shell in the current terminal.
- Check scope and project filters if an expected HostEntry is hidden:

  ```sh
  sshx host list --scope work --project payments --format json
  ```

- Re-run discovery after external edits. Do not assume an old ID, source line, or alias still refers to the same HostEntry.

## Authentication and permissions

**Symptoms:** `sshpass` missing, password rejected, `permission denied`, unsafe password-file warning, or an unreadable config root.

- Key-only inventory and connections do not require `sshpass`. Install it only for password authentication:
  - macOS: `brew install sshpass` from the maintained Homebrew formula.
  - Ubuntu CI/local: `sudo apt-get install --no-install-recommends -y openssh-client sshpass`.
- Keep password-bearing config files owned by the user with mode `0600`. `doctor` reports unsafe permissions without changes by default. Use `sshx doctor --fix-permissions` or Hosts → Doctor → Repair eligible permissions to review one path-and-mode plan and confirm once. Wrong-owner, symlink, shared, and otherwise ineligible paths remain unchanged.
- Keep app-owned runtime and registry directories user-owned with mode `0700`. Do not replace a control socket with a symlink or manually adopt a process.
- Use `IdentityFile`, `ssh-agent`, or an inherited password file descriptor. Passwords must not appear in argv, environment, runtime config, logs, JSON/YAML, or previews.
- A wrong configured password is not retried silently. Supply a corrected secret and run the command again.

In the Doctor TUI, press `R` to display eligible paths with their current and target modes. Press `Y` once to apply that plan, or `N`/`Esc` to cancel without changes. If the terminal cannot display plan content, confirmation stays disabled until you resize it. Results remain in Doctor; use `Tab` and `PgUp`/`PgDn` to scroll every result, then `Esc` to return to Hosts. `sshx tui doctor` opens the same workspace directly. CLI permission repair still requires an interactive confirmation; `--yes`, piped input, and `--no-input` do not authorize unattended chmod.

## Host keys

**Symptoms:** `HOST_KEY_TRUST_REQUIRED`, changed host key, or unexpected known-host path.

- Run the connection interactively once to inspect the OpenSSH fingerprint and explicitly confirm trust.
- For automation, enroll the key through the approved host-key flow first. `--no-input` fails closed on an unknown key.
- Never delete or replace a changed key automatically. Stop and verify the server identity out of band.
- Personal and work scopes use separate known-host stores. A paired VM uses a stable `HostKeyAlias` derived from its immutable VM UUID, not from a temporary transit port.

## Ports and service forwards

**Symptoms:** `SERVICE_PORT_BUSY`, `TRANSIT_BIND`, `FORWARD_BIND`, missing listener, or only part of a multi-forward request opens.

- Check local listeners before retrying. Use a different explicit local port, for example `--forward 5432=15432`.
- Service listeners bind to `127.0.0.1` by default. Non-loopback binds require explicit opt-in for standalone direct tunnels.
- Request all required services in one command with repeated `--forward` options. New listeners roll back as a set when one requested bind fails; existing tunnels and unrelated processes remain untouched.
- Transit ports are temporary app-owned loopback ports. Do not hard-code them in the source config or infer VM success from a listener alone.
- `master responsive`, `listener ready`, and `application health unknown` are separate states. A database or HTTP service still needs its own authorized health check.

## Standalone tunnel lifecycle

**Symptoms:** tunnel missing after the launcher exits, stale status, duplicate tunnel, or stop/restart failure.

```sh
sshx tunnel direct list --format json
sshx tunnel paired list --format json
sshx tunnel direct status <tunnel-id> --format json
```

A standalone tunnel survives the launcher terminal because its detached master and registry state are owned by the tunnel request. Stop it by tunnel ID:

```sh
sshx tunnel direct stop <tunnel-id>
sshx tunnel paired stop <tunnel-id>
```

`sshx` reuses only an exact active request. It does not reconnect automatically. If the control socket no longer responds, status marks the record stale/down and reports cleanup guidance. Do not kill a PID by hand; an unproved process may belong to another request.

## Evidence boundary

CI and local tests use fixtures, temporary HOME/config trees, controlled command paths, local OpenSSH behavior, and release archives. They are **fixture/local validation**. They do not prove access to a real user server.

**User-server validation** is a separate manual step in an authorized environment. Report its server, host-key, authentication, and detached-lifetime results separately. Never convert a fixture pass into a real-server acceptance claim.
