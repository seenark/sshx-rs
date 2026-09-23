# 01: Repair private permissions through doctor

**What to build:** Let users review and repair every safe private-permission finding through `sshx doctor --fix-permissions`, while keeping ordinary `sshx doctor` report-only and preventing newly written private state from needing repair.

**Blocked by:** None (can start immediately).

**Status:** claimed

- [ ] One shared eligibility contract accepts only current-user-owned paths of the expected regular-file or directory type, rejects symlinks and shared paths, and never changes ownership or arbitrary parent directories.
- [ ] Eligible files are repaired to exactly `0600`; eligible directories are repaired to exactly `0700`.
- [ ] Generic readable SSH files without a stored password do not become findings only because other users can read them.
- [ ] `sshx doctor` remains report-only and performs no permission mutation.
- [ ] `sshx doctor --fix-permissions` displays one complete plan and asks for one confirmation before changing any path.
- [ ] Non-interactive execution never applies repairs.
- [ ] Repair continues after individual failures and reports `fixed`, `skipped`, and `failed` for every planned path.
- [ ] Doctor exits nonzero when any repair fails or an unsafe permission finding remains.
- [ ] New private files are created as `0600` and new private directories as `0700`, including application settings regardless of process `umask`.
