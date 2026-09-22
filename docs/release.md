# Build and publish releases

Releases use `vX.Y.Z` tags. GitHub Actions runs the full behavior suite on Ubuntu and macOS before publishing assets.

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
./scripts/package-release.sh 0.1.0 x86_64-unknown-linux-gnu dist
./scripts/verify-release-archive.sh 0.1.0 x86_64-unknown-linux-gnu dist
```

Supported targets:

- `aarch64-apple-darwin` (macOS arm64)
- `x86_64-apple-darwin` (macOS x86_64)
- `aarch64-unknown-linux-gnu` (Linux arm64)
- `x86_64-unknown-linux-gnu` (Linux x86_64)

## Tag publication

1. Update `Cargo.toml` and `Cargo.lock` to the release version.
2. Run formatting, linting, the full behavior suite, and the local package smoke test.
3. Push a `vX.Y.Z` tag.
4. The release workflow builds all four targets, verifies every archive and checksum, then publishes all `.tar.gz` and `.sha256` assets to one GitHub release.
5. Only after publication, the workflow creates fresh isolated mise data, config, cache, state, and HOME directories on Ubuntu and macOS. It runs:

   ```sh
   mise use -g github:seenark/sshx-rs
   mise exec -- sshx --version
   ```

   The expected output is `sshx X.Y.Z` for the tag.

The release job does not publish when either platform's behavior suite or any packaging check fails. The post-publication mise smoke is a required acceptance gate; if it fails, do not announce the release until the asset selection or installation problem is fixed.

## Artifact contract

Each release contains these archives and companion checksums:

```text
sshx-vX.Y.Z-aarch64-apple-darwin.tar.gz
sshx-vX.Y.Z-aarch64-apple-darwin.tar.gz.sha256
sshx-vX.Y.Z-x86_64-apple-darwin.tar.gz
sshx-vX.Y.Z-x86_64-apple-darwin.tar.gz.sha256
sshx-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz
sshx-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz.sha256
sshx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz
sshx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz.sha256
```

Archive root contains executable `sshx`, not a source checkout directory. mise's GitHub backend matches the OS and architecture tokens in each target name and extracts that root binary.

## Validation boundary

Workflow package checks and mise installation are fixture/local validation. They prove local binary execution and GitHub asset selection, not remote user-server authentication. Real server access, first-use host-key enrollment, host-key rotation, and standalone tunnel lifetime need separate authorized manual validation and must not be reported as release evidence unless run.
