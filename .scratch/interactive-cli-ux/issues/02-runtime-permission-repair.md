# 02: Repair blocking permissions during ordinary commands

**What to build:** When an ordinary command is blocked by a safe permission problem, let the user repair that exact path in place and continue the original operation without leaving the command to remember or run `chmod` manually.

**Blocked by:** 01: Repair private permissions through doctor.

**Status:** ready-for-agent

- [ ] A blocked command uses the shared permission-repair eligibility contract instead of defining a second ownership, type, symlink, or privacy policy.
- [ ] The prompt shows the path, current mode, required mode, and reason before any mutation.
- [ ] Consent applies the exact `0600` or `0700` repair and retries the blocked operation once.
- [ ] A failed retry returns the real failure and never enters another repair loop.
- [ ] Declining, `--no-input`, or a missing TTY performs no mutation and returns an actionable error.
- [ ] Wrong ownership, symlinks, wrong path types, shared paths, and arbitrary parent directories are never repaired.
- [ ] When manual repair is valid, the error includes a correctly shell-quoted `chmod` command.
