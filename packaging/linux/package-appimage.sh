#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 7 ]]; then
  echo "usage: $0 VERSION ARCH BINARY LINUXDEPLOY APPIMAGETOOL RUNTIME OUTPUT_DIR" >&2
  exit 2
fi

version="$1"
arch="$2"
binary="$3"
linuxdeploy="$4"
appimagetool="$5"
runtime="$6"
output_dir="$7"

case "$arch" in
  x86_64 | aarch64) ;;
  *)
    echo "unsupported AppImage architecture: $arch" >&2
    exit 2
    ;;
esac

test -x "$binary"
test -x "$linuxdeploy"
test -x "$appimagetool"
test -f "$runtime"
test -f packaging/linux/com.ayi.updater.desktop
test -f packaging/linux/com.ayi.updater.metainfo.xml
test -f assets/icons/updater.png

repo_root="$(pwd)"
binary="$(realpath "$binary")"
linuxdeploy="$(realpath "$linuxdeploy")"
appimagetool="$(realpath "$appimagetool")"
runtime="$(realpath "$runtime")"
output_dir="$(mkdir -p "$output_dir" && cd "$output_dir" && pwd)"
output="$output_dir/updater-$version-linux-$arch.AppImage"
staging="$(mktemp -d)"
trap 'rm -rf -- "$staging"' EXIT

mkdir -p \
  "$staging/AppDir/usr/share/doc/updater" \
  "$staging/AppDir/usr/share/metainfo"
install -m 0644 LICENSE "$staging/AppDir/usr/share/doc/updater/LICENSE"
install -m 0644 README.md "$staging/AppDir/usr/share/doc/updater/README.md"
install -m 0644 THIRD_PARTY_NOTICES.md \
  "$staging/AppDir/usr/share/doc/updater/THIRD_PARTY_NOTICES.md"
install -m 0644 packaging/linux/PORTABLE.md \
  "$staging/AppDir/usr/share/doc/updater/PORTABLE.md"
install -m 0644 packaging/linux/com.ayi.updater.metainfo.xml \
  "$staging/AppDir/usr/share/metainfo/com.ayi.updater.appdata.xml"

(
  cd "$staging"
  NO_STRIP=1 \
    ARCH="$arch" \
    OUTPUT="$output" \
    "$linuxdeploy" --appimage-extract-and-run \
      --appdir AppDir \
      --executable "$binary" \
      --desktop-file "$repo_root/packaging/linux/com.ayi.updater.desktop" \
      --icon-file "$repo_root/assets/icons/updater.png"

  ARCH="$arch" \
    "$appimagetool" --appimage-extract-and-run \
      --runtime-file "$runtime" \
      AppDir \
      "$output"
)

test -x "$output"
