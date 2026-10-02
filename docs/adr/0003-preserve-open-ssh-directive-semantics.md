# ADR 0003: Preserve exact OpenSSH directive semantics

- Status: accepted
- Scope: `sshx` configuration validation and runtime connections

`sshx` treats supported directives inside exact `HostEntry` blocks as OpenSSH directive pass-through. `UserKnownHostsFile` from the selected HostEntry controls known-host storage instead of the sshx default path; direct and standalone connections copy `ProxyCommand` as an opaque command so OpenSSH expands its tokens. `SessionType` is also passed through; `SessionType none` supports transport-only gateway HostEntries because Pair masters already request `-N`. Pair routes continue rejecting `ProxyCommand` because separate gateway and VM masters own the transit route and cannot safely combine it. This keeps the filesystem source of truth and OpenSSH engine authority while making doctor validation match runtime behavior.
