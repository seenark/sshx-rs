# 02: Discover and display exact HostEntries

**What to build:** Let a user point `sshx` at a combined SSH config and list or inspect every exact HostEntry discovered through its Include graph, with source identity and redacted human or machine output.

**Blocked by:** 01: Bootstrap greenfield `sshx` executable.

**Status:** ready-for-agent

- [ ] Include directives are processed at their source position, with relative user paths and wildcard ordering matching OpenSSH semantics.
- [ ] Include cycles terminate with a clear diagnostic instead of looping.
- [ ] The same physical HostEntry reached through multiple paths appears once while retaining every provenance path.
- [ ] Distinct HostEntries remain distinct even when alias, directives, or destination match.
- [ ] `host list` and `host show` expose alias, source, destination, and provenance without exposing password values.
- [ ] Human, JSON, and YAML outputs represent the same data and machine output is exactly one parseable document.
- [ ] Password-bearing domain data cannot be serialized or rendered by public output models.
