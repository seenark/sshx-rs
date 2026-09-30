# 16: Make route-specific Tunnel commands and help agree

**What to build:** A CLI user can run the documented direct and paired Tunnel command forms, including lifecycle commands, and see help that accurately describes the forms the parser accepts. The existing automatic-route Tunnel command remains available.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [x] Documented direct and paired start forms reach the appropriate existing execution path; a route-specific form rejects a HostEntry whose route does not match instead of silently starting a different kind of Tunnel.
- [x] Route-specific list, status, stop, and restart forms operate on persisted Tunnel IDs with the existing ownership checks and do not create a second registry.
- [x] Nested help and setup examples match the parser, and invalid command paths fail with relevant usage rather than displaying help for an unsupported form.

Verification: `skill://implement`, `skill://tdd` (including `tests.md` and `mocking.md`), `skill://code-review`, `skill://diagnosing-bugs`, and `skill://ponytail` were read. The standalone direct lifecycle test fails before this repair with `FORWARD_REQUIRED` masking the expected `TUNNEL_ROUTE_MISMATCH`, then passes after removing the premature missing-forward rejection. Route checks now precede the existing startup forward guards. Wrong-route start/status/stop/restart preserve both the owned master and the persisted registry. `cargo test --test tunnel --test proxycommand` passes all 11 tests; the two `tunnel_help` tests and `invalid_tunnel_paths_return_route_specific_usage` pass. An actual CLI fixture smoke passes direct start/list/status/restart/stop, both start route mismatches without forwards, opposite-route lifecycle rejection, unchanged registry/master, route-filtered lists, and matching-route missing-forward rejection. This is fixture/local validation, not user-server validation.

Review: The previous repair commit `bba4672` passed both prescribed reviewers against `01401d5f59e295c94378abde1ce7431d4deac182`. This lifecycle repair still requires the two parallel reviews against baseline `f569a8a98e0b0c119479eff5adaa194acde389f2`.

Remaining: Main owns the required full suite once after shared integration. The latest full-suite output `artifact://1125` contains this now-repaired standalone lifecycle failure and separate CLI failures. Do not rerun that unchanged full suite merely to confirm it. Keep `ready-for-agent` until the lifecycle repair reviews and required shared verification complete. Unrelated working-tree changes, including `tests/cli.rs`, remain untouched.
