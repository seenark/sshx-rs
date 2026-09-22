# 10: Select and open session service forwards atomically

**What to build:** Let a user optionally select one or more declared services for a direct or paired shell, override local ports for that invocation, and either receive the whole requested forwarding set or none of the new set.

**Blocked by:** 06: Update, rename, and delete HostEntries losslessly; 09: Open a paired VM shell in one terminal.

**Status:** claimed

- [ ] A normal connect opens no app-added service forwarding and asks no forwarding question.
- [ ] Repeated `##PORT` declarations produce unique service choices with loopback destination and matching local defaults.
- [ ] Detailed service metadata can refine destination host and persistent default local port for a declared remote port.
- [ ] Interactive `--bind` supports multi-selection and local-port editing; non-interactive mode uses repeatable deterministic forward arguments.
- [ ] App-added listeners bind to `127.0.0.1` by default.
- [ ] Per-call local overrides do not modify source defaults.
- [ ] All requested local ports are preflighted, and any setup failure tears down only masters and forwards created by that invocation.
- [ ] An unrelated process occupying one requested port remains untouched and yields an interactive correction path or structured non-interactive error.
