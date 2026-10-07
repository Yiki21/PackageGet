# Updater 1.3.0

`1.3.0` is an unsigned cross-platform release. It adds user-visible capability that the 1.2.x line did not have — installing a package by name from a source that cannot search, seeing results while a slow source is still loading, stopping a wait without restarting the app, a background update check, and per-source freshness — and it carries the correctness fixes from two review rounds (issues #14 through #41). It keeps the existing cross-platform asset bundle and one `SHA256SUMS` manifest, but macOS artifacts are now **arm64 (Apple Silicon) only**.

## Highlights

- **Install by name for sources that cannot search.** `bun`, `uv` and `nix-profile` advertise Install but not Search, and `nix-profile` advertises Update but not Updates, so Discover and Updates could never reach those operations. Discover now offers a compact **Install by name** affordance for exactly the configured sources that install without searching, and the Installed inspector offers **Update** for a source that updates without listing updates. The target resolves through the manager's own `install_target`, goes through the same frozen plan, confirmation and grouped-action path as a search result, and blank input is rejected inline.
- **Partial results while a slow source loads, and a way to stop waiting.** Discover used to show a bare `Searching...` until every selected source answered, hiding results that had already arrived and leaving Install, Refresh All and Update All disabled for as long as one source hung. Completed sections now render immediately next to a `Still searching: <names>` line, and both that line and the Updates `loading N remaining` line carry a **Stop waiting** control. Stopping records a neutral per-source `Stopped waiting for X; a late response will be ignored.` notice that offers Retry; late results were already discarded by the request-id guards.
- **A background update check.** The Updates sidebar badge was fed only by the Updates page's lazy count scan, so on a fresh launch the at-a-glance signal could not appear until that page was opened. A read-only count now runs once after the configuration loads and then every five minutes. It never triggers a privileged metadata refresh and never performs a write, it does not trade a known count for the loading glyph and does not slide the status panel open, and when it finds updates it raises the existing native notification under the existing preference.
- **Per-source freshness.** Each Updates source header now shows when it was last checked, the page summary shows the oldest successful check time (or `Not checked yet`), and a source whose refresh failed shows `(last known: N)` instead of a stale count presented as current. Failing sources no longer count toward the page total or the sidebar badge.
- **Light theme meets WCAG AA.** Secondary text, warning/health text and error text were 3.2–4.4:1 on several light-theme surfaces, which failed the 4.5:1 AA floor for the install/update plan's package list, live command lines and the shortcut footer. The tokens now yield 4.6:1 or better on every surface they are painted on, with the muted/secondary/primary hierarchy preserved.

## Fixes

### Round 2 (issues #25–#41)

- A successful install, update or remove no longer forces a privileged metadata sync: the post-operation reload reads the local metadata that the completed write just made authoritative, instead of prompting for authorization a second time and re-downloading indexes (#25).
- Updates show per-source last-checked times, retain a failed source's count only as `last known`, and drop it from the page total and sidebar badge (#41).
- A user-stopped package operation is reported as one neutral line — `Update stopped after X of Y packages` — with Dismiss, instead of being reported as a failure (#35).
- A real failure names the manager with its display name and renders its detail in a bounded, scrollable, copyable monospace block instead of dumping raw stderr inline (#35). Read-path failures keep the error kind and the actionable detail they used to discard (#22).
- The background update check does not hijack the sidebar badge or pop the status panel (#32).
- Empty Discover, Installed and Updates states point at the Package Managers page where the fix lives, instead of asking for a package manager that is not configured (#36).
- The footer and status-panel buttons are named distinctly (History / Show output / Hide output), and a running operation has exactly one reachable Stop control, in the status panel's action row (#39).
- Esc in the search box no longer clears the inspected row, and the footer hints list the controls they were missing: `↑↓ Move`, `Space Select`, `Esc Close` and `Alt+1–5 Pages` (#38).
- A long package step stays visibly alive: the status label carries the current step's elapsed time and the newest command line, instead of a bar frozen at its first fraction (#37).
- Light-theme secondary, warning and error text reach WCAG AA on every surface they are painted on (#40).
- Go search failures name the requirement — an exact module path — instead of a generic command failure, and Go update scanning keeps the healthy tools when one module cannot be resolved (#34, #29).
- Homebrew pinned formulae are no longer offered as updates; they stay visible in the installed inventory (#31).
- apt, dnf and pacman report command failures as failures instead of as empty results, and no longer label rows `Not Installed` when the installed-version lookup itself failed. Exit codes the tools define as "no matches" are still treated as empty results (#28).
- apt update parsing runs in a fixed locale, so a translated `upgradable from` marker can no longer push every row onto the per-row fallback path (#30).
- Command output that merely contains the word `canceled` is no longer reported as a user cancellation; only a cancellation that was actually requested is (#26).
- The streaming runner cannot hang after the child exits: the post-exit output drain is bounded, and a command that leaves a background process holding the inherited pipes is ended rather than waited on forever (#27).
- Failures in one manager no longer abort the whole operation: the remaining managers still run and the first failure is still reported (PR #55, no issue).

### Round 1 (issues #14–#24)

Also included in this release.

- Install is reachable for `bun`, `uv` and `nix-profile`, and Update for `nix-profile`, through **Install by name** and the Installed inspector (#20; see Highlights).
- One slow or hung source no longer hides the other sources' results or disables Install, Refresh All and Update All, and it can be stopped with **Stop waiting** (#23; see Highlights).
- Pacman no longer performs unsupported partial upgrades: a metadata refresh goes through a fixed temporary sync database under the privileged helper, and updates run as one `pacman -Syu`. No code path runs `pacman -Sy` without `--dbpath <temp>` unless `-u` is in the same transaction (#14).
- System writes no longer inherit the GUI's terminal or block on hidden conffile prompts: managed commands run with stdin closed, and the system helper sets `DEBIAN_FRONTEND=noninteractive` with `--force-confdef --force-confold` (#15).
- Install, update and remove are no longer wrapped in 30-second or 90-second timeouts that killed only the direct child and left a write half-applied. Read probes keep their short timeouts (#16).
- Stop on an elevated transaction reports the truth: a privileged command records the cancellation request without signalling the root process group it cannot signal, and its real exit status is reported afterwards (#17).
- DNF builds one name-to-version map from a single `rpm -qa` instead of spawning one `rpm` process per update and per search result, and a package installed in several versions is reported with one clean version string (#18).
- System-category managers can be added, selected and unloaded from the Package Managers page again, including a winget that was missing from `PATH` at first launch (#19).
- Discover can install Snap, Scoop, .NET tool and RubyGems results: those sources report their own install state, so an advertised version no longer makes every result look installed (#21).
- Removing packages shows and executes a frozen plan — package names grouped by manager, an elevation hint, and a count of selected packages hidden by the current filter — instead of confirming a number that could differ from what was removed (#24).
- Read failures show the error kind, the manager message and the failing command with its stderr tail, with a Copy action, instead of a bare `package manager command failed` (#22).

## Platform support change

From `1.3.0`, macOS artifacts are **arm64 (Apple Silicon) only**: `.app.zip` and `.dmg` for arm64. Intel Macs remain supported on `1.2.2`, which stays available under its own immutable tag and its own `SHA256SUMS`; the Intel `.app.zip` and `.dmg` are no longer built for new releases. The repository's macOS contract tests continue to run on a native arm64 runner.

## Release assets

- Linux: Debian `.deb`, RPM `.rpm`, Arch `.pkg.tar.zst`, glibc/musl `.tar.gz`, and glibc `.AppImage` for x86_64/aarch64 as applicable.
- Windows: x86_64 portable `.zip` and per-user setup `.exe`.
- macOS: arm64 (Apple Silicon) `.app.zip` and `.dmg` only.
- Verify every downloaded asset against the matching `SHA256SUMS` file from this release.

## Compatibility and limits

- Existing configuration and Activity history remain readable. This release adds no persistent configuration schema; the Package Manager set and its capabilities are unchanged.
- Windows and macOS artifacts are unsigned. SmartScreen or Gatekeeper may warn on first launch; signing and notarization are intentionally outside the 1.0 policy.
- `cargo audit` still reports four non-blocking advisories for unmaintained or unsound transitive crates in the iced font and renderer dependency chain: `paste`, `rustybuzz`, `ttf-parser`, and `lru`. No fixed upstream versions are currently available for this dependency graph.
- A background update check is read-only and never triggers a privileged refresh, but the first visit to Installed, Updates or Health still performs the required real read-only scans.
- Stopping a search or an update load stops waiting for it; the underlying `list_updates`/`search` call keeps running to its own timeout, and its late result is discarded.
- `pacman -Syu` applies everything newer than the update list showed. The Updates page states this in the confirmation for a pacman update.
- When a Go module's latest-version lookup fails while others succeed, that tool is left out of Updates and the warning is not yet shown in the UI; a scan in which every lookup fails is still reported as an error.
- Go and pipx search resolve one exact module path or distribution name, not a catalog query; the source picker labels them as exact-identifier lookups.

## Previous release

`1.2.2` remains available under its original immutable tag and release assets. This release does not rewrite that history.

---

# Updater 1.2.2

`1.2.2` is an unsigned cross-platform patch release. It fixes Go update discovery when the Go binary directory contains non-executable files such as gup lock files, and keeps the existing RubyGems environment behavior intact.

## Fixes

- Ignores non-executable files in `GOBIN` instead of probing them as Go binaries and failing the entire Go Updates source.
- Keeps executable Go tools discoverable and preserves their module/package identity for updates.
- Adds a regression contract for lock files and other non-executable files in `GOBIN`.
- Refreshes the locked Rust dependency set and removes all currently actionable RustSec vulnerabilities from the release build.

## RubyGems note

RubyGems reports the gems visible to the selected `gem` executable. If the system Ruby and an asdf Ruby are both installed, they have different repositories and can show different update counts. Configure the RubyGems executable explicitly when a specific Ruby installation should be managed.

## Release assets

- Linux: Debian `.deb`, RPM `.rpm`, Arch `.pkg.tar.zst`, glibc/musl `.tar.gz`, and glibc `.AppImage` for x86_64/aarch64 as applicable.
- Windows: x86_64 portable `.zip` and per-user setup `.exe`.
- macOS: arm64/x86_64 `.app.zip` and `.dmg`.
- Verify every downloaded asset against the matching `SHA256SUMS` file from this release.

## Compatibility and limits

- Existing configuration and Activity history remain readable. This patch adds no Package Manager or persistent configuration schema.
- Windows and macOS artifacts are unsigned. SmartScreen or Gatekeeper may warn on first launch.
- `cargo audit` still reports four non-blocking advisories for unmaintained or unsound transitive crates in the iced font and renderer dependency chain: `paste`, `rustybuzz`, `ttf-parser`, and `lru`. No fixed upstream versions are currently available for this dependency graph.

## Previous release

`1.2.1` remains available under its original immutable tag and release assets. This release does not rewrite that history.
