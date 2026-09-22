# 14: Package and verify mise-installable releases

**What to build:** Let a maintainer publish documented macOS and Linux releases that mise can install as an `sshx` executable from `github:seenark/sshx-rs` in a fresh environment.

**Blocked by:** 13: Diagnose configuration and runtime state.

**Status:** resolved

- [x] Repository glossary and ADRs record approved domain terms, OpenSSH engine choice, filesystem source of truth, and tunnel ownership decision.
- [x] Setup and troubleshooting documentation covers roots, Pair setup, direct and paired shells, service forwards, standalone lifecycle, permissions, authentication, host keys, ports, and route failures.
- [x] macOS setup uses the maintained Homebrew `sshpass` formula; release workflow verifies Linux commands before publication when GitHub CI runs.
- [x] Version-tag workflow builds arm64 and x86_64 archives for macOS and Linux with executable `sshx` at archive root.
- [x] Packaging verifier checks a SHA-256 checksum and executable permissions for every release archive before upload.
- [ ] Fresh isolated mise environment discovers the released version, selects the correct platform asset, installs it, and runs `sshx --version` (blocked: GitHub API returns 404 for `github:seenark/sshx-rs`; no released asset exists to test).
- [ ] CI passes the full behavior suite on macOS and Linux before release publication (workflow is implemented; GitHub-hosted run unavailable here).
- [x] Documentation separates fixture/local validation from real user-server results and makes no unsupported acceptance claim.
- [x] No change is made to the deprecated repository; its deprecation notice remains user-owned work.

## Comments

- Local implementation commit: `497176e32429659916cc0eea5880038d1ba6c73e`.
- Manual review covered workflow dependencies, archive root/mode/checksum checks, target naming, mise isolation, domain vocabulary, documentation evidence boundaries, and deprecated-repository isolation. Review terminology findings were corrected before resolution.
- Local proof passed: `mise exec -- cargo fmt --all -- --check`; `mise exec -- cargo check --all-targets --all-features`; `mise exec -- cargo clippy --all-targets --all-features -- -D warnings`; `mise exec -- cargo test --all-targets --all-features`; shell syntax checks; YAML parsing; and macOS arm64 package smoke with checksum, archive-root, mode `0755`, and `sshx --version` assertions.
- External blocker, not verified: `mise ls-remote github:seenark/sshx-rs` returned `mise ERROR HTTP status client error (404 Not Found) for url (https://api.github.com/repos/seenark/sshx-rs/releases)`. No GitHub release, hosted CI run, or `mise use` installation claim is made.

## Answer

497176e32429659916cc0eea5880038d1ba6c73e
