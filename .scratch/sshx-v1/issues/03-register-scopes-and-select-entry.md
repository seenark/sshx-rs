# 03: Register scopes and select one exact HostEntry

**What to build:** Let a user register existing personal and work config roots, filter the catalog by scope or project, and select one exact HostEntry interactively or deterministically from automation.

**Blocked by:** 02: Discover and display exact HostEntries.

**Status:** ready-for-agent

- [ ] Setup registers existing roots without moving files or creating the missing legacy work-root spelling.
- [ ] The current environment can register the combined root and the discovered work root under `~/.private-key/private-key/config`.
- [ ] Entries retain every personal/work and project provenance, and filters match those provenance sets.
- [ ] Running connect without a host opens a searchable selector only in interactive mode.
- [ ] Running without a host under `--no-input` returns `HOST_REQUIRED` without reading stdin.
- [ ] Alias-only selection resolves when unique and returns `HOST_AMBIGUOUS` with candidates when ambiguous.
- [ ] A persistent ID, or alias plus complete source location, resolves one exact entry and rejects partial or mismatched selectors.
