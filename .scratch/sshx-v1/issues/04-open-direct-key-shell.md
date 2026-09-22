# 04: Open a direct private-key shell safely

**What to build:** Let a user select a direct HostEntry and receive an interactive shell through system OpenSSH while preserving existing key and agent authentication, enforcing host-key trust, and cleaning up only the connection `sshx` owns.

**Blocked by:** 03: Register scopes and select one exact HostEntry.

**Status:** resolved

- [x] Runtime configuration contains only the exact selected Host block, excludes all comment metadata, preserves supported trusted directives, and applies system defaults in documented precedence.
- [x] Unsupported wildcard, Match, conditional, global, or token-sensitive semantics fail before OpenSSH configuration evaluation.
- [x] First use displays the OpenSSH fingerprint and records trust only after interactive confirmation.
- [x] `--no-input` fails unknown keys with `HOST_KEY_TRUST_REQUIRED`, and changed keys always stop without replacement.
- [x] Private-key, ssh-agent, and related OpenSSH authentication work without requiring sshpass.
- [x] Authentication success is observed through an owned non-detached control master before the shell opens.
- [x] The remote shell inherits terminal streams and is not wrapped in JSON or YAML.
- [x] Normal exit, SIGINT, SIGHUP, and SIGTERM clean up the owned master without targeting unrelated processes.

## Answer

ed3f993466ec7edcb73856cca2f6f0d2a5e28dca
