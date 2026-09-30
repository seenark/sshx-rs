# 02: Continue incomplete CLI operations in the TUI

**What to build:** A person can deliberately enter the TUI with an operation already selected, or start a valid CLI operation with required information missing and finish that same operation interactively. Fully specified commands continue to execute directly.

**Blocked by:** 01: Browse HostEntries and return from a Session.

**Status:** ready-for-agent

- [ ] Explicit TUI invocation opens either Hosts or a focused, prefilled operation; a connect command missing its HostEntry and a host-show command missing its selector open the appropriate focused selection.
- [ ] Values supplied on the command line remain visible and editable in the TUI; explicit selectors retain exact semantics and never become fuzzy matches or guessed values.
- [ ] Complete CLI commands, help, version, machine output, piped input, and no-input mode never open the TUI. Missing flag values, conflicting options, and malformed commands return actionable CLI errors.
- [ ] Cancelling an unfinished CLI continuation exits with the existing cancellation status and no connection, mutation, listener, or clipboard side effect.

## Verification

The previous `verified` status is withdrawn while ticket-owned continuation regressions from `artifact://761` are repaired. Required checks are editable exact prefill, noninteractive precedence, focused selection, and cancellation without side effects. Failures belonging to other tickets are not evidence that this ticket is complete.
