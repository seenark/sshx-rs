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

Required project-wide verification remains. The prior full suite (`artifact://761`) had 117 failures, including failures outside this ticket. Main owns one final full-suite run after shared integration stabilizes. Keep `ready-for-agent` until that result is reconciled; scoped passes do not imply a full-suite pass.
