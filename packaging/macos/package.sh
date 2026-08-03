#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  printf 'usage: %s VERSION ARCH BINARY OUTPUT_DIR\n' "$0" >&2
  exit 2
fi

version="$1"
arch="$2"
binary="$3"
output_dir="$4"
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
app="$output_dir/Updater.app"
contents="$app/Contents"
resources="$contents/Resources"
iconset="$output_dir/Updater.iconset"
dmg_root="$output_dir/dmg-root"

case "$arch" in
  arm64|x86_64) ;;
  *) printf 'unsupported macOS architecture: %s\n' "$arch" >&2; exit 2 ;;
esac

test -x "$binary"
test ! -e "$app"
test ! -e "$iconset"
test ! -e "$dmg_root"
mkdir -p "$contents/MacOS" "$resources" "$iconset" "$dmg_root"
install -m 0755 "$binary" "$contents/MacOS/updater"
install -m 0644 "$repo_root/LICENSE" "$resources/LICENSE"
install -m 0644 "$repo_root/README.md" "$resources/README.md"

for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$repo_root/assets/icons/updater.png" \
    --out "$iconset/icon_${size}x${size}.png" >/dev/null
  doubled=$((size * 2))
  sips -z "$doubled" "$doubled" "$repo_root/assets/icons/updater.png" \
    --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$resources/Updater.icns"

cat > "$contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>Updater</string>
  <key>CFBundleExecutable</key><string>updater</string>
  <key>CFBundleIconFile</key><string>Updater</string>
  <key>CFBundleIdentifier</key><string>com.ayi.updater</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Updater</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

plutil -lint "$contents/Info.plist"
test "$(plutil -extract CFBundleIdentifier raw "$contents/Info.plist")" = com.ayi.updater
test "$(plutil -extract CFBundleShortVersionString raw "$contents/Info.plist")" = "$version"
[[ "$(file -b "$contents/MacOS/updater")" == *"$arch"* ]]

ditto -c -k --sequesterRsrc --keepParent \
  "$app" "$output_dir/updater-$version-macos-$arch.app.zip"
ditto "$app" "$dmg_root/Updater.app"
ln -s /Applications "$dmg_root/Applications"
hdiutil create \
  -volname Updater \
  -srcfolder "$dmg_root" \
  -ov \
  -format UDZO \
  "$output_dir/updater-$version-macos-$arch.dmg"
