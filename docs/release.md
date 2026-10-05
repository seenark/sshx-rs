# Build and publish releases

Releases use `vX.Y.Z` tags. GitHub Actions runs the full behavior suite on Ubuntu 22.04 and macOS Apple Silicon before publishing assets.
Verification installs the pinned Rust toolchain's `rustfmt` and `clippy` components before running the formatting, linting, and behavior gates.

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

- `aarch64-apple-darwin` (Apple Silicon M1 and later)
- `aarch64-unknown-linux-gnu` (GNU/Linux arm64, Ubuntu 22.04 baseline)
- `x86_64-unknown-linux-gnu` (GNU/Linux x86_64, Ubuntu 22.04 baseline)

Mac Intel, Windows, and Alpine/musl are not supported. Apple Silicon support does not promise compatibility with every historical macOS version.

## Tag publication

1. Ensure `Cargo.toml` and `Cargo.lock` contain the intended release version.
2. Run formatting, linting, the full behavior suite, and the local package smoke test.
3. Push a `vX.Y.Z` tag matching the Cargo package version.
4. The release workflow builds all three targets natively on `macos-14`, `ubuntu-22.04-arm`, and `ubuntu-22.04`. It verifies every archive and checksum, checks that the local asset set exactly matches the six filenames below, and uploads them to a draft GitHub release. It checks that the uploaded asset set exactly matches before making the release stable and latest. Failed uploads or asset-set checks leave a draft, not an incomplete stable release.
5. Only after publication, the workflow creates fresh isolated HOME, XDG config/data, and mise data/config/cache/state directories on all three native platforms. It installs mise without project tools or caches, uses no source checkout or build, and runs from the isolated HOME:

   ```sh
   version="${GITHUB_REF_NAME#v}"
   mise use -g github:seenark/sshx-rs
   selected="$(mise which sshx)"
   case "$selected" in
     "$MISE_DATA_DIR"/installs/*) ;;
     *) printf 'sshx resolved outside isolated mise install: %s\n' "$selected" >&2; exit 1 ;;
   esac
   test -x "$selected"
   test "$(mise exec -- sh -c 'command -v sshx')" = "$selected"
   test "$(mise exec -- sshx --version)" = "sshx $version"
   mise use -g "github:seenark/sshx-rs@$version"
   test "$(mise exec -- sshx --version)" = "sshx $version"
   ```

   Both the default stable installation and pinned installation must report `sshx X.Y.Z` for the tag. The installation smoke proves asset selection and execution, not checksum sidecar verification; the package verifier checks sidecars before upload.

The release job does not publish when either platform's behavior suite or any packaging check fails. The post-publication mise smoke is a required acceptance gate; if it fails, do not announce the release until the asset selection or installation problem is fixed.

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

## Validation boundary

Workflow package checks and mise installation are fixture/local validation. They prove local binary execution and GitHub asset selection, not remote user-server authentication. Real server access, first-use host-key enrollment, host-key rotation, and standalone tunnel lifetime need separate authorized manual validation and must not be reported as release evidence unless run.
