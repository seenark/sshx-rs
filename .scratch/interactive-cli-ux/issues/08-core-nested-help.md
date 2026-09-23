# 08: Provide nested help for root and top-level workflows

**What to build:** Add deterministic command-aware help routing and complete help pages for the root, setup, doctor, and connect workflows, including the new permission and host-action options.

Type: task

Status: resolved

Blocked by: 01, 07

- [x] Root, `setup`, `doctor`, and `connect` support `-h`, `--help`, and `sshx help <command-path>`.
- [x] No arguments and `sshx help` display complete root help.
- [x] Help flags obey position-sensitive semantics: flags before a command path describe the current path, while flags after it describe that path.
- [x] Each page contains applicable purpose, usage, arguments, options, prompt behavior, conflicts, examples, exit behavior, and related commands in the confirmed order.
- [x] Doctor help accurately documents report-only default behavior and `--fix-permissions`.
- [x] Connect help accurately documents exact selectors, interactive selection, every `--action` value, action availability, secret confirmation, and the JSON/YAML conflict.
- [x] Help is plain deterministic text with no color or pager, writes to stdout, and exits `0`.
- [x] `--format` does not alter help rendering.
- [x] Parse errors remain on stderr, exit `2`, and show the nearest relevant usage summary.
- [x] Examples contain no real passwords and use canonical glossary terms.

## Answer

Added static plain-text help pages for root, setup, doctor, and connect. Added shared position-sensitive routing, selector-aware connect help, `--format` ignoring, and nearest-command usage on parser errors. Host, Pair, and tunnel leaf help remains deferred to tickets 09/10.

Evidence:

- `cargo test --test cli` (78 passed)
- `cargo test --test tunnel` (3 passed)
- `cargo test --lib` (15 passed)
- direct `cargo fmt --all -- --check`
- direct `cargo check --all-targets`
- direct `cargo clippy --all-targets --all-features -- -D warnings`
