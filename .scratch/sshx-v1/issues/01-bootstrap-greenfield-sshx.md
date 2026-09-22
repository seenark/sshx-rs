# 01: Bootstrap greenfield `sshx` executable

**What to build:** Create the new Rust project so a maintainer can build and run an `sshx` binary backed by a reusable library, with no source or history imported from the deprecated project.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] The project pins Rust 1.95.0 through mise instead of floating on `latest`.
- [x] The package exposes a reusable library and a thin `sshx` CLI binary without introducing a multi-crate workspace.
- [x] Running `sshx --version` from a local build prints the binary name and package version successfully.
- [x] Formatting, linting, tests, and release compilation run on both macOS and Linux CI.
- [x] No source, test, workflow, annotation compatibility layer, or history is imported from `seenark/sshx-rust`.

## Answer

6fcf61b1ad0b5987eee4b2b176f79689c5dd60a6
