# 14: Package and verify mise-installable releases

**What to build:** Let a maintainer publish documented macOS and Linux releases that mise can install as an `sshx` executable from `github:seenark/sshx-rs` in a fresh environment.

**Blocked by:** 13: Diagnose configuration and runtime state.

**Status:** claimed

- [ ] The repository glossary and ADRs record the approved domain terms, OpenSSH engine choice, filesystem source of truth, and tunnel ownership decision.
- [ ] Setup and troubleshooting documentation covers roots, Pair setup, direct and paired shells, service forwards, standalone lifecycle, permissions, authentication, host keys, ports, and route failures.
- [ ] macOS setup uses the maintained Homebrew sshpass formula, and Linux commands are verified on Linux CI before publication.
- [ ] Version tags produce arm64 and x86_64 archives for macOS and Linux with executable `sshx` at archive root.
- [ ] Every release asset has a published SHA-256 checksum and preserves executable permissions.
- [ ] A fresh isolated mise environment discovers the released version, selects the correct platform asset, installs it, and runs `sshx --version`.
- [ ] CI passes the full behavior suite on macOS and Linux before release publication.
- [ ] Documentation separates fixture/local verification from real user-server results and makes no unsupported acceptance claim.
- [ ] No change is made to the deprecated repository; its deprecation notice remains user-owned work.
