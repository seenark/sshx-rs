#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf 'Usage: %s VERSION TARGET [OUTPUT_DIR]\n' "$0" >&2
}

if [[ $# -lt 2 || $# -gt 3 ]]; then
    usage
    exit 64
fi

version=$1
target=$2
output_dir=${3:-dist}
archive="$output_dir/sshx-v${version}-${target}.tar.gz"
checksum="$archive.sha256"

[[ -f "$archive" ]] || { printf 'missing archive: %s\n' "$archive" >&2; exit 66; }
[[ -f "$checksum" ]] || { printf 'missing checksum: %s\n' "$checksum" >&2; exit 66; }

expected_file=$(awk 'NF {print $2; exit}' "$checksum")
expected_digest=$(awk 'NF {print $1; exit}' "$checksum")
[[ "$expected_file" == "$(basename "$archive")" ]] || {
    printf 'checksum names %s, expected %s\n' "$expected_file" "$(basename "$archive")" >&2
    exit 65
}
if command -v sha256sum >/dev/null 2>&1; then
    actual_digest=$(sha256sum "$archive" | awk '{print $1}')
else
    actual_digest=$(shasum -a 256 "$archive" | awk '{print $1}')
fi
[[ "$expected_digest" == "$actual_digest" ]] || {
    printf 'checksum mismatch for %s\n' "$archive" >&2
    exit 65
}

[[ "$(tar -tzf "$archive")" == 'sshx' ]] || {
    printf 'archive must contain only executable sshx at archive root: %s\n' "$archive" >&2
    exit 65
}

extract_dir=$(mktemp -d "${TMPDIR:-/tmp}/sshx-release-check.XXXXXX")
trap 'rm -rf "$extract_dir"' EXIT
tar -xzf "$archive" -C "$extract_dir"
binary="$extract_dir/sshx"
[[ -x "$binary" ]] || { printf 'archive binary is not executable: %s\n' "$archive" >&2; exit 65; }
mode=$(stat -c '%a' "$binary" 2>/dev/null || stat -f '%Lp' "$binary")
[[ "$mode" == 755 ]] || { printf 'archive binary mode is %s, expected 755\n' "$mode" >&2; exit 65; }
[[ "$($binary --version)" == "sshx $version" ]] || {
    printf 'archive binary version mismatch: %s\n' "$archive" >&2
    exit 65
}
printf 'verified %s (sha256, archive root, mode 755, version %s)\n' "$archive" "$version"
