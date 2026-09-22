#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf 'Usage: %s VERSION TARGET [OUTPUT_DIR]\n' "$0" >&2
    printf 'Targets: aarch64-apple-darwin, x86_64-apple-darwin, aarch64-unknown-linux-gnu, x86_64-unknown-linux-gnu\n' >&2
}

if [[ $# -lt 2 || $# -gt 3 ]]; then
    usage
    exit 64
fi

version=$1
target=$2
output_dir=${3:-dist}

case "$version" in
    ''|*[!A-Za-z0-9.+-]*)
        printf 'invalid release version: %s\n' "$version" >&2
        exit 64
        ;;
esac

case "$target" in
    aarch64-apple-darwin|x86_64-apple-darwin|aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu)
        ;;
    *)
        printf 'unsupported release target: %s\n' "$target" >&2
        exit 64
        ;;
esac

binary="target/$target/release/sshx"
if [[ ! -x "$binary" ]]; then
    printf 'release binary not found: %s\n' "$binary" >&2
    printf 'Build the target before packaging.\n' >&2
    exit 66
fi

mkdir -p "$output_dir"
archive="$output_dir/sshx-v${version}-${target}.tar.gz"
checksum="$archive.sha256"
stage=$(mktemp -d "${TMPDIR:-/tmp}/sshx-release.XXXXXX")
trap 'rm -rf "$stage"' EXIT

install -m 0755 "$binary" "$stage/sshx"
tar -C "$stage" -czf "$archive" sshx

if command -v sha256sum >/dev/null 2>&1; then
    digest=$(sha256sum "$archive" | awk '{print $1}')
else
    digest=$(shasum -a 256 "$archive" | awk '{print $1}')
fi
printf '%s  %s\n' "$digest" "$(basename "$archive")" > "$checksum"
printf 'packaged %s\n' "$archive"
printf 'checksum %s\n' "$checksum"
