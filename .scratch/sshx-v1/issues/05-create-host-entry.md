# 05: Create a HostEntry without restructuring config

**What to build:** Let a user create a new HostEntry in a selected scope, project, folder, and file with a redacted preview and only the minimum Include change required to make it discoverable.

**Blocked by:** 03: Register scopes and select one exact HostEntry.

**Status:** ready-for-agent

- [ ] Interactive and non-interactive commands can specify the target scope and file plus all required managed fields.
- [ ] Every app-created HostEntry receives a unique immutable ID.
- [ ] Preview identifies target files and exact changes while redacting password values.
- [ ] An Include is added only when the target is not already reachable, including through an existing wildcard Include.
- [ ] Existing config structure and unrelated bytes remain unchanged.
- [ ] New files use restrictive permissions and existing files retain their intended mode and line endings.
- [ ] Non-interactive mutation requires prior consent and never opens a confirmation prompt.
- [ ] A changed file identity or digest aborts the write instead of overwriting concurrent external edits.
