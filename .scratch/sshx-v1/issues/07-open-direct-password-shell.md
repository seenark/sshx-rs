# 07: Open a direct password shell without leaking secrets

**What to build:** Let a user connect directly with an existing, prompted, or file-descriptor password while keeping the secret out of process arguments and saving a replacement only after successful authentication and explicit consent.

**Blocked by:** 04: Open a direct private-key shell safely; 06: Update, rename, and delete HostEntries losslessly.

**Status:** resolved

- [x] Existing `##PASSWORD` metadata is read only for the selected HostEntry and is absent from public models and runtime configs.
- [x] sshpass receives exactly one password through an anonymous inherited file descriptor, never argv, environment, or a password file.
- [x] Unrelated child processes and sibling authentication attempts cannot inherit the password descriptor.
- [x] A wrong configured password is not retried automatically; interactive mode requests a new hidden value and identifies the target role.
- [x] `--no-input` performs no prompt, makes at most one configured or supplied password attempt, and returns a structured authentication error.
- [x] A caller can provide a one-use password through an explicit file descriptor without placing it in command arguments.
- [x] A replacement password is written back only after observed authentication success and explicit user confirmation.
- [x] Key-only entries continue to invoke plain OpenSSH and do not require sshpass.

## Answer

2887a42026c1433ea3ec21196891e9f04b29c064
