# 16: Make route-specific Tunnel commands and help agree

**What to build:** A CLI user can run the documented direct and paired Tunnel command forms, including lifecycle commands, and see help that accurately describes the forms the parser accepts. The existing automatic-route Tunnel command remains available.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [x] Documented direct and paired start forms reach the appropriate existing execution path; a route-specific form rejects a HostEntry whose route does not match instead of silently starting a different kind of Tunnel.
- [x] Route-specific list, status, stop, and restart forms operate on persisted Tunnel IDs with the existing ownership checks and do not create a second registry.
- [x] Nested help and setup examples match the parser, and invalid command paths fail with relevant usage rather than displaying help for an unsupported form.

Verification: `skill://implement`, `skill://tdd` (including its test and mocking references), and `skill://code-review` were read. `cargo test --test tunnel --test proxycommand` passes all 11 tests, including the four failures recorded in `artifact://761`. The lifecycle tests now also verify both route filters and that wrong-route status/stop/restart preserve owned masters. The three scoped Tunnel help/invalid-path CLI tests pass. An actual CLI fixture smoke passes explicit direct start, automatic HostEntry aliases `direct`/`paired`, automatic `start`, direct list/status/restart/stop, opposite-route rejection without side effects, paired-start route mismatch, and nested help. This is fixture/local validation, not user-server validation.

Review: The two parallel reviewers prescribed by `skill://code-review` reviewed repair commit `bba4672` against fixed point `01401d5f59e295c94378abde1ce7431d4deac182`. Standards: zero findings. Spec: zero findings. Unrelated working-tree changes were excluded.

Remaining: Run the required full suite once after shared integration. Of the 113 CLI failures in `artifact://761`, four contain the command-path regression. A post-fix scoped run of those four passes `incomplete_tunnel_command_opens_connection_workspace`; the other three now fail later PTY interactions (`interactive_tunnel_command_returns_to_hosts_with_id_and_same_entry`, `interactive_tunnel_startup_failure_cancellation_exits_130_without_listener`, and `tui_reuses_exact_active_tunnel_before_listener_preflight`). Their expectations were not rewritten. Keep `ready-for-agent` until required verification completes.
