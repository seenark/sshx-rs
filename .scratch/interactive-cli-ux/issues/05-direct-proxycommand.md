# 05: Run ProxyCommand for direct sessions and tunnels

**What to build:** Let exact HostEntries use native OpenSSH `ProxyCommand` behavior for direct interactive sessions and direct standalone tunnels, including Cloudflare commands that contain OpenSSH `%` tokens.

**Blocked by:** None (can start immediately).

**Status:** claimed

- [ ] A `ProxyCommand` inside the exact selected HostEntry block is copied into the private runtime configuration without parsing, rewriting, expanding, or executable allowlisting.
- [ ] OpenSSH receives and expands tokens such as `%h`, `%p`, and `%r` for direct interactive connections.
- [ ] The same exact-block behavior works for direct standalone tunnels.
- [ ] Existing host-key policy, authentication, runtime ownership, failure reporting, and cleanup remain intact around the proxied OpenSSH process.
- [ ] Global directives, wildcard Host blocks, and `Host *` do not gain new inheritance behavior.
- [ ] Pair routes still reject `ProxyCommand` on either gateway or VM with an error that identifies the route conflict.
- [ ] `ProxyJump` remains unsupported.
- [ ] A direct HostEntry using `ProxyCommand cloudflared access ssh --hostname %h` is exercised for both an interactive session and a standalone tunnel.
