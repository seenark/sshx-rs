# 13: Create a Pair in one workspace

**What to build:** A person can choose exact gateway and VM HostEntries, resolve their transit route, review both sides and persist one Pair without copying identifiers between commands or navigating a chain of prompt screens.

**Blocked by:** 01: Browse HostEntries and return from a Session; 02: Continue incomplete CLI operations in the TUI.

**Status:** ready-for-agent

- [x] Gateway selection filters ineligible VM choices; duplicate aliases remain distinguishable by source. A partial CLI Pair command keeps its supplied gateway or VM selection visible and editable.
- [x] Ambiguous or missing transit inference shows candidates or accepts explicit transit host and port; no candidate is silently selected.
- [x] One review presents gateway, VM, sources, transit and metadata changes. Consent applies the existing atomic Pair mutation; cancellation or failed validation writes no partial relationship.

## Verification

Read `skill://implement`, `skill://tdd` and its test/mocking references, and `skill://code-review`. Continued the interrupted Pair implementation without discarding unrelated edits. `cargo check --bin sshx` passed. `cargo test --test pairs` passed all eleven CLI/PTY tests, including exact duplicate-source selection, eligible VM filtering, editable supplied gateway/VM, explicit candidate choice, manual transit correction, preview, consent, cancellation, stale-source refusal, invalid explicit selectors, non-interactive execution, and interrupted-mutation recovery. Checks inspect Pair records and preserve unrelated source bytes and permissions; review and results do not reveal stored passwords.

A live 12×46 PTY smoke selected the VM, reviewed the same workspace, scrolled the review, applied one Pair, exited normally, and confirmed the route through `pair list --format json --no-input`. A separate live PTY smoke confirmed that interrupted Pair recovery requires its own consent, restores saved source content, rediscovers the gateway, and permits subsequent setup cancellation without applying a Pair. The recovery regression failed before the fix and passed afterward. This is fixture/local validation, not user-server validation. Commits: `6f329b6` and `41249ce`.

Additional live CLI/PTY smoke verified selected secondary aliases in TUI review, accepted secondary selectors without changing Host alias lists, and confirmed that machine-output operations on a PTY and piped incomplete commands return exit `2` without a workspace or config writes. Pair list retains its existing primary-alias representation.

Required full validation remains blocked by the previously observed unrelated dirty `tests/cli.rs` failures: `expected a literal` in `concat!` at line 4548, missing `fs`/`OpenOptions` imports, and 589 errors in all-target/all-feature check and test runs. Those changes are preserved; the known failing checks were not rerun. Keep `ready-for-agent` until required full validation completes.

## Standards

Parallel review found no documented-standard violations. Two nonblocking judgement calls remain: tuple-based HostEntry/alias selections and repeated selected-VM transit lookup. Existing selection conventions remain; no speculative abstraction was added. The recovery correction introduces no additional standards findings.

## Spec

Parallel review found one safety regression in the interrupted implementation: pending journals had no remaining consent-gated recovery path. Commit `41249ce` restores the existing affected-file review, explicit recovery consent, and rediscovery; the regression test failed before this correction and passed afterward. Follow-up review found no remaining ticket-13 implementation mismatches or scope creep.

Review summary: Standards has zero violations and two nonblocking judgement calls; Spec has zero remaining implementation findings. Required full-suite verification remains blocked as recorded above.
