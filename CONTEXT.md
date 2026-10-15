# sshx v1 glossary

Use these terms in code, documentation, tests, and issue answers. A term describes one boundary and must not be replaced with a looser synonym.

| Approved term | Meaning |
| --- | --- |
| **OpenSSH engine** | The system `ssh` client, control master, forwarding, authentication, and host-key implementation used by `sshx`. `sshx` orchestrates it; it does not replace it. |
| **OpenSSH directive pass-through** | A supported setting inside an exact HostEntry remains governed by OpenSSH, including directive-specific token expansion; `sshx` does not reinterpret its value. |
| **config root** | A registered SSH config file from which OpenSSH `Include` traversal starts. Personal and work roots remain separate. |
| **filesystem source of truth** | The current SSH config files on disk. `sshx` discovers them for every command and never maintains a second host database. |
| **HostEntry** | One exact `Host` block identified by source file and byte span. An alias is a display selector, not an identity. |
| **Pair** | One gateway-to-VM relationship. A Pair names exact HostEntries and owns one transit route. |
| **gateway** | The first OpenSSH hop in a Pair. |
| **VM** | The second OpenSSH hop in a Pair. |
| **session-bound master** | A non-detached OpenSSH control master owned by one interactive shell session. It is cleaned up when that session ends. |
| **Session mode** | User-facing mode that opens an interactive SSH session through a session-bound master. It may have zero service forwards. |
| **standalone tunnel** | A detached OpenSSH master and listener owned by one persisted tunnel request. It can be queried or stopped by tunnel ID from another process. |
| **Tunnel mode** | User-facing mode that creates a standalone tunnel and requires at least one forwarding. |
| **input mode** | The TUI keyboard state: Insert (typed characters go to the search query), Normal (letters are commands), or Select (rows are multi-selected). It is unrelated to Session mode and Tunnel mode. |
| **service forward** | One remote port on the route's last hop mapped to one local port. It may be declared or used ad hoc. |
| **declared service** | A service forward saved in a HostEntry's metadata, with optional default local port, destination host, and name. |
| **usage state** | Per-HostEntry record of last use, use count, and last selected service forwards. It is not a host database; losing it loses only ordering and preselection. |
| **tunnel ownership** | A request owns its control socket, runtime configuration, master process, listeners, and Pair dependencies. Ownership is proved from the locked registry and control socket, never from a PID alone. |
| **fixture/local validation** | Evidence from temporary config trees, fake commands, local OpenSSH, and CI runners. It does not prove access to a user's server. |
| **user-server validation** | Separate manual evidence from an authorized real server. It is never implied by fixture/local validation. |
| **release asset** | A target-specific archive containing executable `sshx` at archive root and a companion SHA-256 checksum. |

## Deliberate vocabulary boundaries

- Say **config root**, not “host database path”.
- Say **filesystem source of truth**, not “cached inventory”.
- Say **Pair**, **gateway**, and **VM**, not “proxy chain”.
- Say **session-bound master** or **standalone tunnel** when ownership differs.
- Say **input mode** for Insert, Normal, and Select. A bare “mode” means Session mode or Tunnel mode.
- Say **listener ready** or **master responsive** only when that evidence exists. Do not call either state application health.
- Say **fixture/local validation** and **user-server validation** as separate evidence classes.
