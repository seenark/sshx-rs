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

Release CI ends after exact-six-asset stable publication; it does not run mise consumer installation checks. Independent download/native archive verification checks the actual published bytes. An isolated local mise installation can separately prove GitHub asset selection and execution without changing the owner's global state or relying on a source build, cached `sshx`, or hand-copied installation. An explicit version pin can install a newly published release immediately; default unpinned latest resolution follows mise's 24-hour release-age filter. These independent checks are not CI gates, and mise installation alone does not verify checksum sidecars.

## Consequences

- Target triples are part of the public artifact contract.
- The archive layout stays independent of source checkout layout.
- Checksums and executable mode are verified before upload.
- Native source and archive gates plus exact-six-asset stable publication determine release CI acceptance. Independent published-archive and local mise evidence is reported separately.
- Real user-server access remains outside CI and is reported separately from fixture/local validation.
