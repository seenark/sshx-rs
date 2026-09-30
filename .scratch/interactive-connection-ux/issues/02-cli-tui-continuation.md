# 02: Continue incomplete CLI operations in the TUI

**What to build:** A person can deliberately enter the TUI with an operation already selected, or start a valid CLI operation with required information missing and finish that same operation interactively. Fully specified commands continue to execute directly.

**Blocked by:** 01: Browse HostEntries and return from a Session.

**Status:** ready-for-agent

- [x] Explicit TUI invocation opens either Hosts or a focused, prefilled operation; a connect command missing its HostEntry and a host-show command missing its selector open the appropriate focused selection.
- [x] Values supplied on the command line remain visible and editable in the TUI; explicit selectors retain exact semantics and never become fuzzy matches or guessed values.
- [x] Complete CLI commands, help, version, machine output, piped input, and no-input mode never open the TUI. Missing flag values, conflicting options, and malformed commands return actionable CLI errors.
- [x] Cancelling an unfinished CLI continuation exits with the existing cancellation status and no connection, mutation, listener, or clipboard side effect.

## Verification

Ticket-owned continuation defects are repaired in `f1bb91a` and `f0a05b2`. On an isolated committed snapshot, `cargo check --offline --bin sshx` passes, 14 picker tests pass, and 25 scoped CLI tests pass: `continuation_`, `selectorless_`, noninteractive precedence, clipboard refusal, Pair copy restrictions, and failed Tunnel startup cancellation. The new exact-selection regression failed before the repair; obsolete action-menu and incidental compiled-config assertions were removed rather than re-pinned.

Actual-binary PTY smoke verifies editable exact alias/ID selection, changed HostEntry selection, a visible Session workspace, cancellation exit `130`, unchanged source bytes, and no sshx runtime/registry/settings state. Complete CLI connect, help, version, and JSON inspection also ran on a usable terminal without opening the TUI. These are fixture/local checks, not user-server validation.

Independent Standards and Spec reviews report no remaining findings. Follow-up review confirms the source-identity and named-outcome cleanup preserves behavior.

Required project-wide verification remains. The latest full suite (`artifact://1035`) finished with exit `101`: 147 CLI tests passed and 66 failed, 26 Hosts tests passed and 2 failed, and 2 Tunnel tests passed and 1 failed. Main owns the final full-suite run after shared integration stabilizes. Keep `ready-for-agent` until required project-wide verification is green; scoped passes do not imply a full-suite pass.

### Second attempt verification

The two ticket-owned CLI failures expected exact selectors to populate fuzzy search text. Exact ID/source/line selection instead prefills the selected HostEntry identity, leaving browsing editable. The two Hosts failures expected the old immediate Session screen and selector validation before non-TTY refusal. These obsolete assertions were removed, not re-pinned. The existing Hosts Session-return scenario remains covered; new binary/PTY regressions cover changing an exact secondary-alias selection by ID or source/line, cancellation `130` without OpenSSH or persisted state, prefix/ambiguous/conflicting selector errors, and piped TUI refusal. No runtime change was needed.

Current-worktree proof passes: `cargo check --offline --bin sshx`; 10 CLI `continuation_` tests; 12 CLI `selectorless_` tests; exact noninteractive-precedence and unknown-ID tests; 2 new Hosts continuation tests; and the existing Hosts direct-Session return test. This is 24 scoped CLI tests and 3 scoped Hosts tests. Actual-binary PTY smoke also observes cancellation without SSH/state, a shell receiving `secondary`, session-bound master cleanup, restored selection, and direct complete connect/help/root version/JSON inspection without TUI. Source bytes remain unchanged. This is fixture/local validation only.

Of the failures in `artifact://1035`, 64 CLI failures and the Tunnel route-mismatch assertion remain unresolved by this ticket. Hosts browsing/refresh/terminal handoff belongs to ticket 01; forwarding and startup/retry scenarios to tickets 03–06; registered Tunnel lifecycle/status/reuse to ticket 07; host actions and update/rename/delete to tickets 08/10/11; Pair inspection/setup/recovery to tickets 12/13; Doctor review to ticket 15; and route-specific command errors to ticket 16. Several fail before reaching their behavior because they expect obsolete PTY screens; this ticket does not claim those scenarios pass. `direct_session_allow_bind_applies_to_remote_socks_and_specific_local_forwards` and `standalone_direct_tunnel_survives_launcher_and_stops_by_id` require their owners to distinguish incidental assertions from real forwarding/route defects.

Commit `7db08e0` contains the isolated Hosts regression changes. Deleting `explicit_tui_id_prefill_can_search_to_another_host` and `explicit_tui_source_prefill_can_choose_another_duplicate_host` remains within the existing shared `tests/cli.rs` worktree changes: neither test exists in `HEAD`, so those deletions cannot be committed independently without including unrelated work. No ignored file was force-added, and no unchanged failing full suite was rerun.

Second-attempt independent review: Standards reports no documented-standard breaches or actionable smells; Spec reports no missing requirements, wrong behavior, or scope creep in the isolated commit and supplemental CLI deletions. Both axes have zero findings. The remaining blocker is Main's required green project-wide run, not an unverified ticket-owned runtime change.
