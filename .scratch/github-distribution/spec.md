# GitHub distribution for sshx-rs

Status: ready-for-agent

## Problem Statement

The project is currently a local Git repository without a GitHub remote. The owner wants to publish it as the public repository `seenark/sshx-rs`, using their authenticated `gh` account, and distribute prebuilt binaries through GitHub Releases so users can install with `mise use -g github:seenark/sshx-rs`.

The supported platforms are macOS on Apple Silicon, starting with M1, and GNU/Linux on arm64 and x86_64. Mac Intel and Windows are not supported. Linux uses Ubuntu 22.04 as its compatibility baseline.

Existing CI, release packaging, checksum verification, installation documentation, and an accepted release-asset contract already cover much of this work. However, the current contract still includes Mac Intel and assumes four archives. It must be narrowed consistently rather than supplemented with a second release implementation.

## Solution

Publish the existing project to a public GitHub repository owned by `seenark`. Adapt the existing GitHub Actions workflows and packaging tools to the three supported targets. Run the existing behavior checks before publication, and publish versioned archives with companion SHA-256 checksums when a matching version tag is pushed.

Users install the latest stable release with `mise use -g github:seenark/sshx-rs` and run the executable named `sshx`. No custom mise plugin, source compilation, or installation options are required. The selected first published release uses package version `0.1.2` and tag `v0.1.2`. The existing `v0.1.0` and `v0.1.1` tags remain immutable; workflow validation and Linux ARM64 compilation failures prevented their publication. Further matching patch versions and new tags are authorized if needed to complete this release verification.

Release acceptance uses the existing highest-level distribution seam: install the actual published GitHub release with mise in an isolated environment and run `sshx --version`. Extend this check to all three supported OS/architecture pairs. Do not modify the owner's global mise configuration during verification.

## User Stories

1. As the repository owner, I want a public GitHub repository named `seenark/sshx-rs`, so that users can access the project and its releases.
2. As the repository owner, I want publication to use my existing authenticated `gh` account, so that no additional credentials or account setup are required.
3. As a contributor, I want the existing source and Git history published on the main branch, so that I can inspect the implementation and its evolution.
4. As a maintainer, I want the existing CI and release tools reused, so that there is one maintained distribution path.
5. As a macOS user with an M1 or later Apple Silicon Mac, I want a native arm64 executable, so that I can run sshx without Intel emulation.
6. As a maintainer, I want Mac Intel excluded from release builds and documented support, so that the support contract matches the owner's decision.
7. As a Linux x86_64 user, I want a native GNU/Linux release asset, so that I can install without building Rust source.
8. As a Linux arm64 user, I want a native GNU/Linux release asset, so that I can install on arm64 machines without cross-compilation.
9. As an Ubuntu user, I want the Linux binaries to run on Ubuntu 22.04 and later compatible releases, so that installation does not require upgrading to Ubuntu 24.04.
10. As a maintainer, I want Windows excluded from this release pipeline, so that unsupported platform builds do not imply Windows support.
11. As a maintainer, I want Alpine and musl excluded from this scope, so that the existing GNU/Linux contract is not expanded without a separate decision.
12. As a user, I want to install globally with `mise use -g github:seenark/sshx-rs`, so that the normal mise workflow manages sshx.
13. As a user, I want mise to select the correct release asset automatically, so that I do not need platform-specific installation options.
14. As a user, I want an unpinned installation to select a published stable release, so that normal installation does not depend on draft or prerelease artifacts.
15. As a user, I want to pin an existing release version through mise, so that I can retain a known version when newer releases become available.
16. As a user, I want the installed command to remain `sshx`, so that the GitHub repository name does not change the CLI interface.
17. As a maintainer, I want version tags to initiate releases, so that ordinary branch pushes do not publish new stable versions.
18. As a maintainer, I want a release tag to match the Cargo package version, so that the published version and executable output agree.
19. As a maintainer, I want formatting, linting, and the existing behavior suite to pass before publication, so that a failing project is not distributed as a stable release.
20. As a maintainer, I want each supported target to build and execute its packaged binary on a native runner, so that cross-platform packaging errors are found before upload.
21. As a user, I want each release asset to contain exactly one executable `sshx` at archive root, so that mise can extract and expose the command reliably.
22. As a user, I want the extracted executable to retain mode `0755`, so that installation produces a runnable command.
23. As a user, I want a companion SHA-256 checksum for each archive, so that downloaded release assets can be verified.
24. As a maintainer, I want all three archives and their checksums present in the release, so that no supported platform is missing its installation artifact.
25. As a maintainer, I want publication blocked when any supported target fails its build or package verification, so that users do not receive a partial platform release.
26. As a maintainer, I want the published release installed through the same plain mise command users run, so that verification covers real GitHub asset selection and extraction.
27. As the repository owner, I want verification to use isolated mise directories, so that acceptance checks do not replace my global tool configuration.
28. As a user, I want `sshx --version` to report the installed release version, so that I can confirm which binary mise installed.
29. As a user, I want installation and release documentation to list the same supported targets as the actual assets, so that I do not mistake Mac Intel, Windows, or musl for supported platforms.
30. As a user, I want the system OpenSSH engine and optional sshpass prerequisites documented, so that I understand which runtime tools are not bundled in the release.
31. As a maintainer, I want failed post-publication installation checks reported as release-acceptance failures, so that publication alone is not mistaken for a verified release.
32. As a maintainer, I want fixture/local validation distinguished from user-server validation, so that release checks do not imply access to a real remote server.

## Implementation Decisions

- Create a public repository owned by `seenark`, named `sshx-rs`, using the existing authenticated GitHub CLI account. Connect the local repository through an origin remote and publish the main branch with its existing source and history. Do not rewrite history or remove unrelated tracked content as part of this feature.
- Reuse the existing CI workflow, release workflow, release-packaging helper, archive-verification helper, package smoke helper, and installation documentation. Do not introduce another release framework, custom mise plugin, or installation service.
- The release target set is exactly `aarch64-apple-darwin`, `aarch64-unknown-linux-gnu`, and `x86_64-unknown-linux-gnu`. Mac Intel, including `x86_64-apple-darwin`, is removed rather than retained as a deprecated target.
- Keep the existing native macOS arm64 packaging runner where supported. Use explicit Ubuntu 22.04 native runner labels for the two Linux targets: `ubuntu-22.04` and `ubuntu-22.04-arm`. The macOS hardware contract is Apple Silicon M1 and later; this conversation does not establish support for every historical macOS version installed on those machines.
- Keep the existing pinned Rust toolchain and locked release builds. Do not change application dependencies or the executable name to implement distribution.
- Keep ordinary push and pull-request verification separate from release publication. Stable releases are triggered by `vX.Y.Z` tags, consistent with the existing version contract; the selected tag-triggered approach does not require publishing arbitrary non-version `v*` tags.
- Validate that each release tag matches the Cargo package version. The selected first published release is `v0.1.2`; retain all earlier tags unchanged and create a new matching patch tag if a tagged implementation needs correction. Use the matching current package version rather than publishing a mismatched tag.
- Preserve the existing release asset interface: a target-specific gzip-compressed tar archive, named with the executable name, v-prefixed version, and Rust target triple; exactly one executable `sshx` at archive root; executable mode `0755`; and one companion SHA-256 sidecar per archive.
- Update every target-dependent interface consistently: the workflow build matrix, package helper target allowlist, package smoke helper target allowlist, publication asset count, post-publication installation matrix, accepted release-asset contract, and user-facing support documentation. The complete release contains three archives and three checksum sidecars, not the previous four-plus-four set.
- Retain existing behavior-suite and package-verification gates. A failed verification or failed supported-target package prevents publication. Supply explicit GitHub repository context when publishing from a job that has no source checkout, so release creation does not depend on implicit local Git discovery.
- Publish the complete asset set as one stable GitHub release. Do not expose an incomplete stable release while assets are still being built. Failed post-publication installation verification blocks release acceptance and announcement; it does not imply that the already-published release was automatically removed.
- Verify the default GitHub backend identifier `github:seenark/sshx-rs` without asset-pattern overrides or platform-specific options. Preserve version pinning through the same backend.
- Run post-publication mise installation on all three supported native platform pairs. Isolate HOME and mise configuration, data, cache, and state directories. Installation verification must not depend on an earlier source build or a previously installed sshx executable.
- Preserve the runtime boundary: sshx orchestrates the system OpenSSH engine. Release archives do not bundle OpenSSH or sshpass, and sshpass remains necessary only for password-authentication paths.
- Update the existing accepted release-asset decision to reflect the explicit removal of Mac Intel. Preserve its archive-layout, checksum, and validation-boundary decisions. Do not create a second contradictory platform contract or add a new glossary term for this distribution change.

## Testing Decisions

- Use one primary acceptance seam: the published GitHub release consumed through a fresh mise installation, followed by execution of the installed CLI. This is the existing release workflow's post-publication seam, extended to cover macOS arm64, Linux arm64, and Linux x86_64.
- A good test verifies consumer-visible behavior: the plain install command succeeds on a supported platform, selects the correct native asset, exposes executable `sshx`, and reports the expected version. Do not test workflow source text, copied configuration values, internal function names, or incidental runner defaults.
- Use the existing package-verification seam before publication. It already checks archive root layout, companion checksum identity and digest, executable mode, and executable version by extracting and running the built artifact. Reuse these checks rather than duplicating them in a new unit-test layer.
- Exercise Linux installation and execution on Ubuntu 22.04 for both architectures. A build on a newer Linux runner is not sufficient proof of the agreed compatibility baseline. Other Linux distributions remain uncertified unless exercised separately.
- Use native Apple Silicon execution for macOS acceptance. Removing Mac Intel means no Intel build or Intel installation gate is required.
- Verify the complete published asset set externally: three target archives and their three checksum sidecars, with no Mac Intel or Windows release asset. Do not replace this with source-text assertions about a matrix.
- Verify release failure boundaries through the existing workflow gates and helpers: mismatched tag/package versions must not publish, and unsupported packaging targets must be rejected rather than silently packaged. Add a focused permanent behavior check only if an existing uncertain boundary requires one; do not add a second distribution test framework.
- Keep the existing formatting, linting, and application behavior suite as pre-publication gates on macOS and Linux. Application SSH behavior is unchanged by this feature and does not require another set of application tests.
- Confirm that version pinning resolves and executes the same initial published version through mise. Use isolated state for this check as well as for the default latest-stable install.
- Preserve the domain's validation boundary. These checks are fixture/local validation using local machines and GitHub runners. They do not prove user-server validation, real-server authentication, host-key enrollment or rotation, or standalone tunnel lifetime.
- No additional test seam needs user clarification: the proposed acceptance seam is the already-established post-publication mise installation check. The latest request asks for specification synthesis without another interview.

## Out of Scope

- macOS on Intel, Intel release assets, Rosetta-based compatibility, and universal macOS binaries.
- Windows builds, Windows runtime support, or Windows release assets.
- Alpine Linux, musl targets, static-musl packaging, and guarantees for Linux systems older than the Ubuntu 22.04 compatibility baseline.
- Certification of other Linux distributions or every historical macOS release.
- Homebrew distribution, Cargo registry publication, custom mise plugins, shell installers, or another package manager.
- New application features, changes to HostEntry discovery or mutation, changes to Pair behavior, or changes to session-bound master and standalone tunnel ownership.
- Bundling OpenSSH or sshpass inside the release archive.
- Selecting a new license, changing project ownership, rewriting existing Git history, deleting unrelated tracked files, or adding signing/notarization infrastructure.
- Release publication on every main-branch update, automatic version bumping, or a separate prerelease channel.
- Editing the owner's actual global mise configuration as part of acceptance verification.
- Implementing or publishing the repository and releases during this specification-only task.

## Further Notes

- User decisions: public repository; release publication on version-tag pushes; Ubuntu 22.04 or later compatible Linux baseline; macOS Apple Silicon M1 and later only. The latest hardware decision replaces the earlier four-target proposal, not the Linux architecture coverage.
- The existing accepted decision at `docs/adr/0002-mise-release-assets.md` currently includes Mac Intel and four archives. This specification explicitly revises that target set to three. Implementation must update that existing decision and its consumers together; no compatibility shim is required.
- Existing prior art is present in `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `scripts/package-release.sh`, `scripts/verify-release-archive.sh`, `scripts/test-release-package.sh`, `docs/release.md`, and the installation section of `docs/setup.md`.
- Read-only checks observed an active `seenark` GitHub account with repository/workflow access, a clean local main branch, no Git remote, installed gh/cargo/rustc/mise commands, and Cargo version `0.1.0`. The repository lookup did not resolve `seenark/sshx-rs`. These are implementation starting facts, not evidence that publication or installation has already succeeded; recheck mutable prerequisites when implementing.
- An inactive GitHub account reported an authentication failure. The active account is `seenark`; do not change or repair unrelated accounts for this feature.
- mise's documented GitHub backend supports release-asset platform matching, extracted executables, stable version selection, checksums, and version pinning: [mise GitHub backend](https://mise.jdx.dev/dev-tools/backends/github.html).
- Native runner labels and supported architectures are documented by GitHub: [GitHub-hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners). Avoid the obsolete Mac Intel runner in the current workflow; deleting the Intel target removes the need to replace it.
- GitHub release creation supports explicit repository selection and existing-tag verification: [gh release create](https://cli.github.com/manual/gh_release_create).
- This task publishes a ready-for-agent specification only. No repository creation, source/workflow edits, pushes, tags, releases, builds, application tests, or mise installation are performed as part of writing it.
