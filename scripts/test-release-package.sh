#!/usr/bin/env bash
set -euo pipefail

version=${1:-$(cargo metadata --no-deps --format-version=1 | python3 -c 'import json, sys; print(json.load(sys.stdin)["packages"][0]["version"])')}
target=${2:-$(rustc -vV | awk '/^host:/{print $2}')}
output_dir=${3:-"$(mktemp -d "${TMPDIR:-/tmp}/sshx-release-smoke.XXXXXX")"}

case "$target" in
    aarch64-apple-darwin|aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu)
        ;;
    *)
        printf 'release smoke supports macOS/Linux targets only, got %s\n' "$target" >&2
        exit 64
        ;;
esac

cargo build --locked --release --target "$target"
./scripts/package-release.sh "$version" "$target" "$output_dir"
./scripts/verify-release-archive.sh "$version" "$target" "$output_dir"
printf 'release smoke passed for %s/%s\n' "$version" "$target"
