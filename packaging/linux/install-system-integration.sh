#!/bin/sh
set -eu

if [ "$(id -u)" -ne 0 ]; then
  echo "run this installer as root" >&2
  exit 1
fi

unset CDPATH
script_dir="$(cd "$(dirname -- "$0")" && pwd)"

install -d -m 0755 \
  /usr/lib/updater \
  /usr/share/icons/hicolor/scalable/apps \
  /usr/share/polkit-1/actions
install -m 0755 \
  "$script_dir/updater-system-helper" \
  /usr/lib/updater/updater-system-helper
install -m 0644 \
  "$script_dir/com.ayi.updater.policy" \
  /usr/share/polkit-1/actions/com.ayi.updater.policy
install -m 0644 \
  "$script_dir/updater.svg" \
  /usr/share/icons/hicolor/scalable/apps/updater.svg

echo "Updater system integration installed."
