# 04: Open a direct private-key shell safely

**What to build:** Let a user select a direct HostEntry and receive an interactive shell through system OpenSSH while preserving existing key and agent authentication, enforcing host-key trust, and cleaning up only the connection `sshx` owns.

**Blocked by:** 03: Register scopes and select one exact HostEntry.

**Status:** ready-for-agent

- [ ] Runtime configuration contains only the exact selected Host block, excludes all comment metadata, preserves supported trusted directives, and applies system defaults in documented precedence.
- [ ] Unsupported wildcard, Match, conditional, global, or token-sensitive semantics fail before OpenSSH configuration evaluation.
- [ ] First use displays the OpenSSH fingerprint and records trust only after interactive confirmation.
- [ ] `--no-input` fails unknown keys with `HOST_KEY_TRUST_REQUIRED`, and changed keys always stop without replacement.
- [ ] Private-key, ssh-agent, and related OpenSSH authentication work without requiring sshpass.
- [ ] Authentication success is observed through an owned non-detached control master before the shell opens.
- [ ] The remote shell inherits terminal streams and is not wrapped in JSON or YAML.
- [ ] Normal exit, SIGINT, SIGHUP, and SIGTERM clean up the owned master without targeting unrelated processes.
