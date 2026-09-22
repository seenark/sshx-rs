# 06: Update, rename, and delete HostEntries losslessly

**What to build:** Let a user update, rename, or delete one exact HostEntry while preserving every unrelated byte and refusing stale, referenced, or active targets.

**Blocked by:** 05: Create a HostEntry without restructuring config.

**Status:** claimed

- [ ] Updates patch only the selected block and preserve comments, ordering, whitespace, CRLF/LF style, trailing newline, and unrelated HostEntries byte-for-byte.
- [ ] Rename changes the requested alias while preserving the entry ID.
- [ ] Delete removes only the selected block and never cascades to other entries.
- [ ] Delete succeeds only for an unreferenced entry with no proven active managed use.
- [ ] Every mutation reindexes current files and reports broken or stale selection state instead of guessing.
- [ ] Symlink, ownership, inode, digest, and concurrent-edit checks fail closed before commit.
- [ ] Multi-file mutation reports partial recovery state honestly if rollback cannot restore every file.
- [ ] Human, JSON, and YAML results contain no password value or raw secret-bearing block.
