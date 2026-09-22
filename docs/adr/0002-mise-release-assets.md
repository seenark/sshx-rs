# ADR 0002: mise-compatible release assets

- Status: accepted
- Scope: `sshx` distribution

## Decision

Version tags use `vX.Y.Z`. Each tag publishes four target archives:

- `sshx-vX.Y.Z-aarch64-apple-darwin.tar.gz`
- `sshx-vX.Y.Z-x86_64-apple-darwin.tar.gz`
- `sshx-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz`
- `sshx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`

Each archive contains exactly one executable named `sshx` at archive root with mode `0755`. Each archive has a companion `.sha256` file. GitHub's mise backend uses target names to select the matching OS and architecture without a repository-specific plugin.

Release publication depends on the full behavior suite on Ubuntu and macOS. A fresh, isolated mise data/config/cache/state directory then installs the published GitHub release and runs `sshx --version`.

## Consequences

- Target triples are part of the public artifact contract.
- The archive layout stays independent of source checkout layout.
- Checksums and executable mode are verified before upload.
- A release is not accepted from local packaging alone; the post-publication mise smoke test must pass.
- Real user-server access remains outside CI and is reported separately from fixture/local validation.
