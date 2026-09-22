# 02: Discover and display exact HostEntries

**What to build:** Let a user point `sshx` at a combined SSH config and list or inspect every exact HostEntry discovered through its Include graph, with source identity and redacted human or machine output.

**Blocked by:** 01: Bootstrap greenfield `sshx` executable.

**Status:** resolved

- [x] Include directives are processed at their source position, with relative user paths and wildcard ordering matching OpenSSH semantics.
- [x] Include cycles terminate with a clear diagnostic instead of looping.
- [x] The same physical HostEntry reached through multiple paths appears once while retaining every provenance path.
- [x] Distinct HostEntries remain distinct even when alias, directives, or destination match.
- [x] `host list` and `host show` expose alias, source, destination, and provenance without exposing password values.
- [x] Human, JSON, and YAML outputs represent the same data and machine output is exactly one parseable document.
- [x] Password-bearing domain data cannot be serialized or rendered by public output models.

## Answer

e40c482c95064c341d89f8a4b1595800335be6b0
