# 09: Create a HostEntry from the TUI or incomplete CLI

**What to build:** A person can create an exact HostEntry through one editable TUI workspace, beginning either from Hosts or a CLI create operation that provided only some required fields.

**Blocked by:** 01: Browse HostEntries and return from a Session; 02: Continue incomplete CLI operations in the TUI.

**Status:** ready-for-agent

- [x] The workspace selects an existing config root, scope and project as applicable, destination file, alias and host destination; supplied CLI values are prefilled and editable, while optional password input remains masked.
- [x] Preview shows the intended filesystem change without applying it; applying the reviewed plan requires consent and preserves existing private-file and mutation safety rules.
- [x] Missing required values in non-interactive mode fail with actionable errors. Cancellation and invalid target paths do not create a HostEntry or modify an unrelated file.

Verification: `skill://implement` was read with its TDD and two-axis code-review references. The 13 current `host_create` CLI/PTY regressions and `empty_hosts_setup_cancel_keeps_explicit_config_available_for_create` pass. An independent fixture/PTY smoke runs the actual binary through Hosts `Ctrl+N`, persistent post-create Hosts feedback, partial CLI create with an edited supplied alias, selection of the work/payments config root without changing the personal root, optional user/port, masked password and redacted review, explicit TUI preview without writes, and review cancellation with exit `130`. Standards and Spec reviews report zero findings.

The shared effort test compile blockers are repaired in the working tree: restore `fs`/`OpenOptions` imports and the VM `HostName` string inside the Pair fixture's `concat!`. The outdated create test expecting an explicit config/scope conflict to enter the form is removed; the existing rejection test passes. Only ticket09 behavior coverage and create documentation are staged; unrelated effort test changes and the Pair fixture remain unstaged.

Remaining verification blocker: unwrapped `cargo test --no-fail-fast -- --test-threads=1` completes with exit `101`: 244 passed, 117 failed. Library/binary unit tests, Doctor, Hosts, and Pair integration targets pass; CLI has 123 passed/113 failed, ProxyCommand has 7 passed/1 failed, and Tunnel has 0 passed/3 failed. All 13 create regressions, empty-Hosts creation, and private password-file rejection pass in this run. Failures include unrelated CLI/TUI continuation, old PTY expectations, Pair workspaces/recovery, and tunnel command/lifecycle contracts. Full output: `artifact://761`. Ticket remains `ready-for-agent` because required project-wide verification is not green; unrelated regressions are not changed by ticket09. The earlier claim that Hosts creation is unreachable is stale; current `run_hosts` calls the shared create workspace and returns to Hosts after success.
