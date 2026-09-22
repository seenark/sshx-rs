# 01: Bootstrap greenfield `sshx` executable

**What to build:** Create the new Rust project so a maintainer can build and run an `sshx` binary backed by a reusable library, with no source or history imported from the deprecated project.

**Blocked by:** None (can start immediately).

**Status:** claimed

- [ ] The project pins Rust 1.95.0 through mise instead of floating on `latest`.
- [ ] The package exposes a reusable library and a thin `sshx` CLI binary without introducing a multi-crate workspace.
- [ ] Running `sshx --version` from a local build prints the binary name and package version successfully.
- [ ] Formatting, linting, tests, and release compilation run on both macOS and Linux CI.
- [ ] No source, test, workflow, annotation compatibility layer, or history is imported from `seenark/sshx-rust`.
