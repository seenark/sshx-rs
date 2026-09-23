# 08: Provide nested help for root and top-level workflows

**What to build:** Add deterministic command-aware help routing and complete help pages for the root, setup, doctor, and connect workflows, including the new permission and host-action options.

**Blocked by:** 01: Repair private permissions through doctor; 07: Choose host actions interactively and copy stored passwords safely.

**Status:** ready-for-agent

- [ ] Root, `setup`, `doctor`, and `connect` support `-h`, `--help`, and `sshx help <command-path>`.
- [ ] No arguments and `sshx help` display complete root help.
- [ ] Help flags obey position-sensitive semantics: flags before a command path describe the current path, while flags after it describe that path.
- [ ] Each page contains applicable purpose, usage, arguments, options, prompt behavior, conflicts, examples, exit behavior, and related commands in the confirmed order.
- [ ] Doctor help accurately documents report-only default behavior and `--fix-permissions`.
- [ ] Connect help accurately documents exact selectors, interactive selection, every `--action` value, action availability, secret confirmation, and the JSON/YAML conflict.
- [ ] Help is plain deterministic text with no color or pager, writes to stdout, and exits `0`.
- [ ] `--format` does not alter help rendering.
- [ ] Parse errors remain on stderr, exit `2`, and show the nearest relevant usage summary.
- [ ] Examples contain no real passwords and use canonical glossary terms.
