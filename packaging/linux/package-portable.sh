#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 5 ]]; then
  echo "usage: $0 VERSION ARCH LIBC BINARY OUTPUT_DIR" >&2
  exit 2
fi

version="$1"
arch="$2"
libc="$3"
binary="$4"
output_dir="$5"

case "$arch" in
  x86_64 | aarch64) ;;
  *)
    echo "unsupported portable architecture: $arch" >&2
    exit 2
    ;;
esac

case "$libc" in
  glibc | musl) ;;
  *)
    echo "unsupported libc variant: $libc" >&2
    exit 2
    ;;
esac

test -x "$binary"
test -f LICENSE
test -f README.md
test -f packaging/linux/PORTABLE.md

package_name="updater-$version-linux-$arch-$libc"
archive="$output_dir/$package_name.tar.gz"
staging="$(mktemp -d)"
trap 'rm -rf -- "$staging"' EXIT

mkdir -p "$output_dir" "$staging/$package_name"
install -m 0755 "$binary" "$staging/$package_name/updater"
install -m 0644 LICENSE "$staging/$package_name/LICENSE"
install -m 0644 README.md "$staging/$package_name/README.md"
install -m 0644 packaging/linux/PORTABLE.md "$staging/$package_name/PORTABLE.md"

tar \
  --sort=name \
  --mtime='UTC 1970-01-01' \
  --owner=0 \
  --group=0 \
  --numeric-owner \
  -C "$staging" \
  -czf "$archive" \
  "$package_name"

mapfile -t archive_files < <(tar -tzf "$archive" | sort)
expected_files=(
  "$package_name/"
  "$package_name/LICENSE"
  "$package_name/PORTABLE.md"
  "$package_name/README.md"
  "$package_name/updater"
)
mapfile -t expected_files < <(printf '%s\n' "${expected_files[@]}" | sort)
test "${archive_files[*]}" = "${expected_files[*]}"
