# 13: Diagnose configuration and runtime state

**What to build:** Give the user one `doctor` command that checks local prerequisites, config integrity, Pair references, host-key storage, port readiness, and managed runtime state without repairing or terminating anything silently.

**Blocked by:** 08: Create and validate a one-to-one Pair; 12: Manage standalone paired tunnels.

**Status:** resolved

- [x] Doctor distinguishes missing/unreadable roots, Include problems, unsupported semantics, duplicate IDs, broken Pair references, and stale selections.
- [x] It checks OpenSSH for every installation and requires sshpass only when password authentication is configured or requested.
- [x] It warns about password-bearing files with unsafe permissions but never chmods them automatically.
- [x] It reports personal and work known-hosts paths and detects missing or insecure app-owned state locations.
- [x] It distinguishes local port conflicts, gateway route configuration, managed master state, listeners, and unproven application health.
- [x] Stale registry or socket state is reported without adopting or killing processes whose ownership cannot be proved.
- [x] Human diagnostics provide actionable stage-specific guidance, while JSON and YAML return the same structured findings.
- [x] Fixture/local results are labeled separately from any real user-server validation.

## Answer

6ea0b1ccab2d6db93aca205efc557bbed695319b
