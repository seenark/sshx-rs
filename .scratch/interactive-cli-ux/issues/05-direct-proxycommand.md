# 05: Run ProxyCommand for direct sessions and tunnels

**What to build:** Let exact HostEntries use native OpenSSH `ProxyCommand` behavior for direct interactive sessions and direct standalone tunnels, including Cloudflare commands that contain OpenSSH `%` tokens.

Type: task

Status: resolved

Blocked by: none (can start immediately)

- [x] A `ProxyCommand` inside the exact selected HostEntry block is copied into the private runtime configuration without parsing, rewriting, expanding, or executable allowlisting.
- [x] OpenSSH receives and expands tokens such as `%h`, `%p`, and `%r` for direct interactive connections.
- [x] The same exact-block behavior works for direct standalone tunnels.
- [x] Existing host-key policy, authentication, runtime ownership, failure reporting, and cleanup remain intact around the proxied OpenSSH process.
- [x] Global directives, wildcard Host blocks, and `Host *` do not gain new inheritance behavior.
- [x] Pair routes still reject `ProxyCommand` on either gateway or VM with an error that identifies the route conflict.
- [x] `ProxyJump` remains unsupported.
- [x] A direct HostEntry using `ProxyCommand cloudflared access ssh --hostname %h` is exercised for both an interactive session and a standalone tunnel.

## Answer

The direct compiler now captures `ProxyCommand` arguments from the raw directive before inline-comment handling, preserving `#` and OpenSSH `%h`, `%p`, and `%r` tokens unchanged. Direct sessions and standalone tunnels keep this behavior; Pair routes still reject `ProxyCommand`, and `ProxyJump` remains unsupported.

Evidence:

- `cargo test --test proxycommand` — 7 passed.
- `cargo test --test cli direct_connect_compiles_exact_block_and_uses_owned_master -- --exact` — passed.
- `cargo test --all-targets --all-features` — passed.
- `cargo check --all-targets`, clippy with `-D warnings`, and `cargo fmt --all -- --check` — passed.
