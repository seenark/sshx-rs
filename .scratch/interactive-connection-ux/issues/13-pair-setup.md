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

The prior compiler blockers are resolved. The latest integrated full-suite run (`artifact://1404`) compiled and recorded 28 CLI failures, including eleven legacy Pair setup/recovery cases; all other targets passed. A new integrated full-suite run remains required after the ticket-owned changes below. Main owns that run; keep `ready-for-agent` until it completes successfully.

## Standards

Parallel review found no documented-standard violations. Two nonblocking judgement calls remain: tuple-based HostEntry/alias selections and repeated selected-VM transit lookup. Existing selection conventions remain; no speculative abstraction was added. The recovery correction introduces no additional standards findings.

## Spec

Parallel review found one safety regression in the interrupted implementation: pending journals had no remaining consent-gated recovery path. Commit `41249ce` restores the existing affected-file review, explicit recovery consent, and rediscovery; the regression test failed before this correction and passed afterward. Follow-up review found no remaining ticket-13 implementation mismatches or scope creep.

Historical review summary: Standards has zero violations and two nonblocking judgement calls; Spec has zero remaining implementation findings. Required integrated full-suite verification remains pending.

### New authorized run

Read `skill://implement`, `skill://tdd` and its test/mocking references, `skill://code-review`, `skill://ponytail`, and `skill://diagnosing-bugs`. No production bug was reproduced. Removed eleven obsolete or duplicate dirty CLI tests rather than restoring removed prompts or the standalone recovery command. Preserved transit cardinality and mismatch scenarios in `pair_setup_requires_explicit_transit_for_ambiguous_candidates`. Equivalent current-interface checks live in `tests/pairs.rs`; unique checks now cover selectorless duplicate gateway and VM sources with secondary aliases, Pairs-tab review cancellation and refreshed records, stale sources before and after review with editable values, malformed-journal refusal, and recovery lock contention after consent.

`cargo check --offline --bin sshx` passed. `cargo test --offline --test pairs` passed all thirteen tests; `cargo test --offline --test cli pair_setup -- --nocapture` passed all seven selected tests. Current Pair list output continues using primary aliases; tests verify selected secondary aliases in the workspace and preserve source alias lists rather than requiring a changed machine schema.

Independent live binary smoke used a real 120×40 PTY: Hosts → Pairs → setup selected exact duplicate gateway/VM sources through secondary aliases, explicitly chose the second transit, left every source unchanged through review, applied only after Enter consent, refreshed the Pairs workspace, and exited normally. Pair list JSON and reciprocal source metadata confirmed the chosen relationship; decoy files remained unchanged. Separate live PTY smoke verified that `--yes` cannot bypass recovery consent, declining preserves source/journal/backups, consenting while another writer holds the lock refuses without writes, successful recovery restores source content and rediscovery, and subsequent setup cancellation exits `130` without Pair metadata. Another live smoke refused a stale source before review while retaining selected fields; malformed-journal machine preview returned exit `2` without writes. These are fixture/local checks, not user-server validation.

Commit `3fc9fb0` contains only reviewed ticket-owned CLI hunks, Pair tests, and this tracked ticket. An isolated archive of that commit passed all thirteen `tests/pairs.rs` tests and all six committed `pair_setup` CLI tests, proving the commit does not rely on unrelated dirty tests. Temporary smoke fixtures, the isolated tree, and the staging patch were removed.

#### Standards

Parallel committed-diff review found zero hard violations. One nonblocking possible Duplicated Code smell notes repeated successful-apply assertions in the new Pairs-tab test. These assertions exercise a distinct root-workspace transition and exact duplicate-source relationship; the small repetition does not justify another helper.

#### Spec

Parallel committed-diff review found zero findings. Exact duplicate-source gateway/VM selection, secondary aliases, review cancellation, explicit apply, reciprocal metadata, untouched decoys and alias lists, refreshed records, transit boundaries, preview, recovery consent and lock refusal, malformed-journal refusal, and stale-source refusal retain current-interface coverage. No production change or scope expansion was required.

Review summary: Standards has zero hard violations and one nonblocking judgement call; Spec has zero findings. Remaining verification is Main's integrated full-suite run; status remains `ready-for-agent`.
