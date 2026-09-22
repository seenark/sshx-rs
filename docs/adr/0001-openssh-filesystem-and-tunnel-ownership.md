# ADR 0001: OpenSSH engine, filesystem source of truth, and tunnel ownership

- Status: accepted
- Scope: sshx v1 runtime and configuration

## Context

`sshx` must preserve existing OpenSSH configuration, support exact gateway/VM Pair routes, and manage both interactive sessions and standalone tunnels. A second host database would become stale when a user edits SSH files with another tool. PID-only process tracking cannot prove which request owns a master or listener.

## Decision

1. Use the system **OpenSSH engine** for connections. `sshx` compiles narrow runtime configuration, starts OpenSSH control masters, and observes OpenSSH evidence. `sshx` does not implement an SSH protocol client.
2. Treat SSH config files on disk as the **filesystem source of truth**. Discover config roots and their `Include` graph for each command. Store only registered roots and managed runtime metadata; never copy hosts into an application-owned database.
3. A Pair owns exact gateway and VM HostEntries. The gateway master creates the transit route; the VM master consumes it. A direct connection does not create a gateway.
4. A session-bound master belongs only to its interactive session and is cleaned up in reverse dependency order. A standalone tunnel owns its detached masters, control sockets, runtime files, listeners, and Pair dependencies. The registry records this ownership under a lock, and a control-socket check proves liveness. A PID by itself is never an ownership identity.
5. Exact active standalone requests may reuse their own persisted tunnel ID. Different source identities, routes, or forwarding requests never share ownership implicitly.

## Consequences

- External edits are visible on the next discovery and can produce diagnostics instead of silent repair.
- OpenSSH options, authentication, host-key policy, forwarding, and platform behavior remain the system's responsibility.
- Runtime artifacts require private directories and cleanup paths for normal exits and signals.
- A stale registry or dead control socket is reported as stale; `sshx` does not adopt or kill an unproved process.
- Fixture/local validation and user-server validation remain separate evidence classes.

## Rejected alternatives

- A custom SSH protocol engine would duplicate OpenSSH security and platform behavior.
- A host database would violate filesystem source-of-truth semantics.
- PID-only tracking would allow a later process to be mistaken for an owned master.
- Shared global masters would cross tunnel ownership boundaries.
