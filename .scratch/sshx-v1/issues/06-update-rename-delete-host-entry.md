# 06: Update, rename, and delete HostEntries losslessly

**What to build:** Let a user update, rename, or delete one exact HostEntry while preserving every unrelated byte and refusing stale, referenced, or active targets.

**Blocked by:** 05: Create a HostEntry without restructuring config.

**Status:** resolved

- [x] Updates patch only the selected block and preserve comments, ordering, whitespace, CRLF/LF style, trailing newline, and unrelated HostEntries byte-for-byte.
- [x] Rename changes the requested alias while preserving the entry ID.
- [x] Delete removes only the selected block and never cascades to other entries.
- [x] Delete succeeds only for an unreferenced entry with no proven active managed use.
- [x] Every mutation reindexes current files and reports broken or stale selection state instead of guessing.
- [x] Symlink, ownership, inode, digest, and concurrent-edit checks fail closed before commit.
- [x] Multi-file mutation reports partial recovery state honestly if rollback cannot restore every file.
- [x] Human, JSON, and YAML results contain no password value or raw secret-bearing block.

## Answer

c0d32cd87cc9d822ea43149b92c4ea3e70b4987f
