# 05: Create a HostEntry without restructuring config

**What to build:** Let a user create a new HostEntry in a selected scope, project, folder, and file with a redacted preview and only the minimum Include change required to make it discoverable.

**Blocked by:** 03: Register scopes and select one exact HostEntry.

**Status:** resolved

- [x] Interactive and non-interactive commands can specify the target scope and file plus all required managed fields.
- [x] Every app-created HostEntry receives a unique immutable ID.
- [x] Preview identifies target files and exact changes while redacting password values.
- [x] An Include is added only when the target is not already reachable, including through an existing wildcard Include.
- [x] Existing config structure and unrelated bytes remain unchanged.
- [x] New files use restrictive permissions and existing files retain their intended mode and line endings.
- [x] Non-interactive mutation requires prior consent and never opens a confirmation prompt.
- [x] A changed file identity or digest aborts the write instead of overwriting concurrent external edits.

## Answer

c29dd938748383a66877077d1816d1683cf5e460
