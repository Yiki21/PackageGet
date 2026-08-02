#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
test -n "$version"

if [[ "${GITHUB_REF:-}" == refs/tags/* ]]; then
  test "${GITHUB_REF_NAME#Build-v}" = "$version"
fi

for package in updater updater-manager-api updater-managers updater_core; do
  awk -v package="$package" -v expected="$version" '
    $0 == "name = \"" package "\"" { found = 1; next }
    found && /^version = / {
      matched = ($0 == "version = \"" expected "\"")
      exit
    }
    END { exit !(found && matched) }
  ' Cargo.lock
done

rpm_version="${version/-/\~}"
test "$(sed -n 's/^version = "\([0-9].*~beta\.[0-9]*\)"/\1/p' ui/Cargo.toml)" = "$rpm_version"

arch_version="${version/-/}"
(
  source packaging/arch/PKGBUILD
  test "$pkgver" = "$arch_version"
  test "$_upstream_version" = "$version"
)
grep -Fxq $'\t'"pkgver = $arch_version" packaging/arch/.SRCINFO
grep -Fxq $'\t'"source = updater-$arch_version.tar.gz::https://github.com/Yiki21/PackageGet/archive/refs/tags/Build-v$version.tar.gz" packaging/arch/.SRCINFO

grep -Fxq "# Updater $version" RELEASE_NOTES.md
grep -Fq "\`$version\` is an unsigned Linux preview." README.md

printf 'release metadata is consistent for %s\n' "$version"
