# 05: Run ProxyCommand for direct sessions and tunnels

**What to build:** Let exact HostEntries use native OpenSSH `ProxyCommand` behavior for direct interactive sessions and direct standalone tunnels, including Cloudflare commands that contain OpenSSH `%` tokens.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] A `ProxyCommand` inside the exact selected HostEntry block is copied into the private runtime configuration without parsing, rewriting, expanding, or executable allowlisting.
- [x] OpenSSH receives and expands tokens such as `%h`, `%p`, and `%r` for direct interactive connections.
- [x] The same exact-block behavior works for direct standalone tunnels.
- [x] Existing host-key policy, authentication, runtime ownership, failure reporting, and cleanup remain intact around the proxied OpenSSH process.
- [x] Global directives, wildcard Host blocks, and `Host *` do not gain new inheritance behavior.
- [x] Pair routes still reject `ProxyCommand` on either gateway or VM with an error that identifies the route conflict.
- [x] `ProxyJump` remains unsupported.
- [x] A direct HostEntry using `ProxyCommand cloudflared access ssh --hostname %h` is exercised for both an interactive session and a standalone tunnel.

## Answer

Integrated backend commit `9b34bb8c332975d6a3aa7c781118be8f3b9eb364` as `3505abe`. Exact HostEntry `ProxyCommand` text now reaches direct session and standalone tunnel runtime configs unchanged, including OpenSSH `%` tokens; Pair routes reject the directive with a route conflict, while `ProxyJump` remains unsupported.

Evidence from backend verification:

- `cargo test --test proxycommand` — 6 passed.
- `cargo test --lib connect` — 7 passed.
- Full suite — 71 passed across 7 suites.
- `cargo check --all-targets`, clippy with `-D warnings`, and `cargo fmt --all -- --check` — passed.
