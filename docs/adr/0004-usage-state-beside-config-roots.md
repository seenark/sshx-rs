# ADR 0004: Usage state beside config roots

- Status: accepted
- Scope: sshx TUI ordering and service forward preselection
- Clarifies: ADR 0001 decision 2

## Context

The TUI orders HostEntries by recent use and frequency, shows last use, and preselects the service forwards used last time so `⇥⏎` connects without typing ports. That needs per-HostEntry usage state. ADR 0001 forbids an application-owned host database and limits sshx-owned storage to registered roots and managed runtime metadata. Writing usage into SSH config on every connect would make each connect a config mutation.

## Decision

1. Usage state is stored in one sidecar file per config root, next to the root file and named after it like the existing mutation lock (`.<root-name>.sshx-usage.json`), with mode `0600`.
2. It holds only last use time, use count, and the last selected service forwards per HostEntry. It never holds aliases for display, hostnames, users, or other host fields, and discovery never reads hosts from it.
3. A HostEntry is keyed by its `##SSHX ID` when present, otherwise by its first alias and source path relative to the root. When sshx mutates a HostEntry for another reason and it lacks `##SSHX ID`, the mutation adds one and usage moves to the new key. Connecting never writes SSH config.
4. A HostEntry in an included file belongs to the root that included it first.
5. If the sidecar cannot be written, sshx skips recording usage; the connection is unaffected.

## Consequences

- Deleting or losing the sidecar loses only ordering and preselection.
- Renaming an alias or moving a block without an `##SSHX ID` drops its usage.
- The sidecar syncs with the user's SSH config if that directory is version-controlled; concurrent edits on two machines may conflict in that file.

## Rejected alternatives

- Writing usage or last-used forwards into SSH config comments on connect: turns every connect into a reviewed mutation with lock and failure paths.
- One central file under `~/.config/sshx/`: separates usage from the roots it describes and from the user's existing sync.
- Writing `##SSHX ID` on first connect: violates "connecting never writes SSH config".
