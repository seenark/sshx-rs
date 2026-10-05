# ADR 0002: mise-compatible release assets

- Status: accepted
- Scope: `sshx` distribution

## Decision

Version tags use `vX.Y.Z`. Each tag publishes three target archives:

- `sshx-vX.Y.Z-aarch64-apple-darwin.tar.gz`
- `sshx-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz`
- `sshx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`

Each archive contains exactly one executable named `sshx` at archive root with mode `0755`. Each archive has a companion `.sha256` file, for exactly six release assets. GitHub's mise backend uses target names to select the matching OS and architecture without a repository-specific plugin.

Supported platforms are Apple Silicon M1 and later and GNU/Linux arm64/x86_64 with an Ubuntu 22.04 baseline. Mac Intel, Windows, and Alpine/musl are not supported. Archives do not bundle the system OpenSSH engine or optional `sshpass`; `sshpass` is needed only for password-authentication paths.

Release publication depends on the full behavior suite on Ubuntu 22.04 and macOS Apple Silicon. Packaging runs natively on all three supported platforms and verifies each archive and checksum. Publication requires the exact six-file set locally, uploads a draft, and checks the exact uploaded set before making the release stable and latest.

After publication, fresh isolated HOME, XDG config/data, and mise data/config/cache/state directories install the published GitHub release on all three native platforms. Both the default stable installation and version pinning must execute `sshx` with the expected version. Executable selection must resolve inside the isolated mise installation, without a source build or cached `sshx`. This installation smoke does not itself verify checksum sidecars.

## Consequences

- Target triples are part of the public artifact contract.
- The archive layout stays independent of source checkout layout.
- Checksums and executable mode are verified before upload.
- A release is not accepted from local packaging alone; the post-publication mise smoke test must pass.
- Real user-server access remains outside CI and is reported separately from fixture/local validation.
