# Build and publish releases

Releases use `vX.Y.Z` tags. Ordinary CI and tag verification run natively on Ubuntu 22.04 x86_64 (`ubuntu-22.04`), Ubuntu 22.04 arm64 (`ubuntu-22.04-arm`), and macOS Apple Silicon (`macos-14`) before publishing assets.
Verification installs the pinned Rust toolchain's `rustfmt` and `clippy` components before running the formatting, linting, full behavior suite, and native package smoke gates on each platform.

## Local package smoke test

Build and verify the current host target:

```sh
mise exec -- ./scripts/test-release-package.sh
```

The smoke test checks:

- target-specific archive name;
- exactly one `sshx` file at archive root;
- executable mode `0755` after extraction;
- SHA-256 digest and checksum filename;
- `sshx --version` equals the package version.

Package a previously built target directly:

```sh
./scripts/package-release.sh 0.1.3 x86_64-unknown-linux-gnu dist
./scripts/verify-release-archive.sh 0.1.3 x86_64-unknown-linux-gnu dist
```

Supported targets:

- `aarch64-apple-darwin` (Apple Silicon M1 and later)
- `aarch64-unknown-linux-gnu` (GNU/Linux arm64, Ubuntu 22.04 baseline)
- `x86_64-unknown-linux-gnu` (GNU/Linux x86_64, Ubuntu 22.04 baseline)

Mac Intel, Windows, and Alpine/musl are not supported. Apple Silicon support does not promise compatibility with every historical macOS version.

## Tag publication

1. Ensure `Cargo.toml` and `Cargo.lock` contain the intended release version.
2. Run formatting, linting, the full behavior suite, and the local package smoke test.
3. Push a `vX.Y.Z` tag matching the Cargo package version.
4. The release workflow builds all three targets natively on `macos-14`, `ubuntu-22.04-arm`, and `ubuntu-22.04`. It verifies every archive and checksum, checks that the local asset set exactly matches the six filenames below, and uploads them to a draft GitHub release. It checks that the uploaded asset set exactly matches before making the release stable and latest. Failed uploads or asset-set checks leave a draft, not an incomplete stable release.

The release job does not publish when any platform's verification gates or any packaging check fails. Release CI ends after stable publication of the exact six assets; it does not run mise consumer installation checks. The mise action in verification and packaging still supplies the pinned Rust toolchain.

## Artifact contract

Each release contains exactly three archives and three companion checksums:

```text
sshx-vX.Y.Z-aarch64-apple-darwin.tar.gz
sshx-vX.Y.Z-aarch64-apple-darwin.tar.gz.sha256
sshx-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz
sshx-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz.sha256
sshx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz
sshx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz.sha256
```

Archive root contains executable `sshx`, not a source checkout directory. mise's GitHub backend matches the OS and architecture tokens in each target name and extracts that root binary.

Release archives do not bundle the system OpenSSH engine or optional `sshpass`. Install OpenSSH before use; `sshpass` is needed only for password-authentication paths.

## Independent consumer validation

GitHub Release assets are the distribution source. After publication, download the native archive and its checksum with `gh release download`, then run `scripts/verify-release-archive.sh VERSION TARGET DOWNLOAD_DIR` on that native platform. This checks the actual published bytes, archive layout, mode, checksum, and executable version rather than a locally built substitute.

An isolated local mise installation can separately check GitHub asset selection and execution. Use a fresh temporary HOME, XDG config/data, and mise config/data/cache/state directories, and run outside the source checkout. Install through `github:seenark/sshx-rs`, check that `mise which sshx` resolves inside the isolated installation, and execute `mise exec -- sshx --version`. Do not copy binaries into mise's cache or installation directories; that does not exercise a real GitHub backend installation. Neither independent check is a CI gate, and neither changes the owner's global mise configuration.

By default, unpinned latest resolution excludes releases younger than 24 hours, so `mise use -g github:seenark/sshx-rs` may select an older eligible stable release or find none immediately after publication. An explicit `github:seenark/sshx-rs@X.Y.Z` pin bypasses the age filter and can install the newly published version immediately. Keep the default age policy; see mise's [minimum release age security policy](https://mise.jdx.dev/security.html#minimum-release-age) and [setting reference](https://mise.jdx.dev/configuration/settings.html#minimum_release_age). Installation checks prove asset selection and execution, not checksum sidecar verification.

## Validation boundary

Workflow package checks and mise installation are fixture/local validation. They prove local binary execution and GitHub asset selection, not remote user-server authentication. Real server access, first-use host-key enrollment, host-key rotation, and standalone tunnel lifetime need separate authorized manual validation and must not be reported as release evidence unless run.
