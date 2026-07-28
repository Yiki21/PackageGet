# Updater UI & Functionality Plan

> Status: Completed
> Updated: 2026-07-28

## Goals

- Preserve user context across refreshes and package operations.
- Make long-running operations understandable, recoverable, and auditable.
- Improve keyboard efficiency and desktop-native interaction patterns.
- Keep the current Apple-inspired, dopamine-accented visual system coherent.
- Add features incrementally without destabilizing package-manager execution.

## Working principles

- Never retry privileged or destructive operations automatically.
- Preserve the current page, filters, search, sorting, and inspector selection whenever valid.
- Distinguish a configuration reload from a post-operation data refresh.
- Use semantic colors and shared components instead of page-local styling.
- Represent partial success explicitly; do not report a batch as fully successful when one manager failed.
- Keep external actions such as opening a URL or sending a notification user-initiated or configurable.
- Verify every phase with formatting, compilation, and focused behavior tests.

## Phase 1 — Preserve UI context

**Status:** Completed (2026-07-27)

### Scope

- Introduce explicit reload reasons:
  - application startup;
  - configuration changed;
  - package operation completed.
- Preserve source selections during post-install, post-update, and post-removal refreshes.
- Retain selections only when their package manager is still configured.
- Preserve page-local search and sort state.
- Preserve the Installed inspector selection only while the package remains available.
- Keep configuration changes authoritative by pruning removed package managers.

### Acceptance criteria

- Updating packages does not clear the selected update sources.
- Removing packages does not clear the selected installed sources.
- Installing packages keeps Discover source selection and reruns the previous query.
- Saving configuration removes selections belonging to disabled managers.
- No stale selected package remains after a data reload.
- `cargo fmt --all -- --check`, `cargo check -p updater`, and focused tests pass.

### Delivered

- Added explicit startup, configuration-change, and package-operation reload reasons.
- Post-operation reloads preserve valid source selections, page-local search, and sorting.
- Startup selects all configured Updates sources while keeping Installed lazy-loaded.
- Installed sources selected before reload automatically reload their full package lists after counts finish.
- Removed managers and stale selected packages are pruned safely.
- The Installed inspector remains selected only while its package still exists.
- Added focused reload-policy tests; all 3 tests pass.

## Phase 2 — Completed operation outcomes

**Status:** Completed (2026-07-27)

### Scope

- Add a shared `OperationOutcome` model for install, update, and removal.
- Track action, package count, manager count, success/failure state, and concise error detail.
- Keep a compact completion summary visible after work finishes.
- Preserve Activity details after completion until dismissed or superseded.
- Prepare the model for partial success and targeted retry.

### Acceptance criteria

- Users can confirm what happened after an operation completes.
- Full success, partial failure, failure, and cancellation have distinct states.
- Activity logs remain accessible after completion.
- Starting a new operation cleanly supersedes the previous compact summary.

### Delivered

- Added a shared `OperationOutcome` for install, update, and removal.
- Batch execution now records package and source totals plus partial completion before failure.
- Added a compact success/failure summary that stays visible until dismissed.
- Preserved the completed operation's Activity logs after active work finishes.
- Added explicit Activity and Dismiss controls to the completion panel.
- Added focused outcome-summary tests; all 5 project UI tests pass.

## Phase 3 — Settings draft and dirty state

**Status:** Completed (2026-07-27)

### Scope

- Separate editable Settings state from the active persisted configuration.
- Derive and display `is_dirty`.
- Disable Save when there are no changes.
- Add Revert.
- Prompt before leaving Settings or closing the window with unsaved changes.
- Apply the draft to package operations only after a successful save.

### Acceptance criteria

- Unsaved changes are always visible.
- Revert restores the last saved configuration.
- Leaving Settings cannot silently discard changes.
- Package-manager operations never use an unsaved executable path.

### Delivered

- Settings now edits an isolated draft with a persisted baseline.
- Unsaved edits no longer mutate the active package-manager configuration.
- Save is disabled while clean and applies the draft only after persistence succeeds.
- Added an `Unsaved changes` indicator and Revert action.
- Navigation away from Settings and window close requests now require Save, Discard, or Cancel.
- Added focused draft/baseline tests; the full workspace passes 70 tests (8 UI + 62 core).

## Phase 4 — Recoverable errors and retry

**Status:** Completed for source operations (2026-07-27); failed package-plan retry continues in Phase 5

### Scope

- Replace plain error text with a shared error-state component.
- Add per-manager retry for initialization, search, load, and refresh failures.
- Map classified errors to useful guidance:
  - authorization denied;
  - package database locked;
  - executable missing or invalid;
  - network/transient failure.
- Retain failed operation context for targeted retry.

### Acceptance criteria

- Every visible recoverable error provides an appropriate next action.
- Retry runs only the failed or explicitly selected scope.
- Privileged and destructive actions never retry automatically.

### Delivered

- Added a shared non-color-only error card with title, detail, warning mark, and Retry action.
- Discover can retry search for only the failed source.
- Installed and Updates can retry initialization/load failures for only the failed source.
- Retrying clears stale error state and prevents duplicate concurrent requests.
- Classified command guidance remains visible in the error detail.
- Failed package-plan retry is deferred to the shared Phase 5 preflight/plan model.
- Added a global thin scrollbar style and a 16px Settings scroll safety area so right-edge buttons no longer touch the drag rail.

## Phase 5 — Update All with preflight

**Status:** Completed (2026-07-27)

### Scope

- Add `Update all available` as a separate action from `Refresh all`.
- Refresh and build a grouped execution plan before confirmation.
- Show package counts per manager, failed scans, exclusions, and privilege requirements.
- Execute manager groups sequentially.
- Report partial results and allow retry of failed groups.

### Acceptance criteria

- Users can inspect exactly what Update All will do before execution.
- Failed or stale managers are never silently included.
- Partial completion is represented accurately.

### Delivered

- Added a distinct `Update All Available` action; it is not conflated with Refresh All.
- Update All force-refreshes every configured source before building a frozen plan.
- The confirmation card shows package/source counts, authorization guidance, and named failed sources that are excluded.
- Zero-update preflight shows a non-actionable `No updates found` result.
- Concurrent refresh, selected-update, retry, and preflight actions are guarded from overlap.
- Batch outcomes retain the exact failed source and partial package progress.
- Failed updates offer `Re-scan Failed Source`; the source is refreshed and a new confirmation plan is built before retry.
- Failed-source retry plans are scoped to that source and cannot repeat already-successful sources.
- Added focused preflight/plan tests; the full workspace passes 72 tests (10 UI + 62 core).

## Phase 6 — Navigation summaries and badges

**Status:** Completed (2026-07-27)

### Scope

- Show the known update count in the Updates sidebar item.
- Show loading and source-failure states without relying on color alone.
- Show a dirty-state indicator for Settings.
- Add page summary chips for freshness, selected scope, visible count, and failures.

### Acceptance criteria

- Sidebar badges are useful and limited; no badge storm.
- Unknown, loading, stale, and current data are visually distinct.
- Summary counts match the currently visible/filterable data.

### Delivered

- Updates navigation shows a bounded count, loading state, or failure warning with deterministic priority.
- Settings navigation shows a lightweight dirty-state dot.
- Navigation status uses plain text/symbols with no capsule badge background.
- Discover, Updates, and Installed show flat `color dot + text` summaries instead of rounded chips.
- Page summaries expose result/update/package totals, current source scope, selection count, and source failures.
- Added badge priority/count tests; all 12 UI tests pass.
- The full Core suite had one transient crates.io network failure and passed when the affected online test was retried; workspace compilation and diff checks pass.

## Phase 7 — Package inspector actions

**Status:** Completed (2026-07-27)

### Scope

- Extract the Installed details pane into a reusable package inspector.
- Reuse it in Discover and Updates.
- Add explicit `Open homepage`, `Copy URL`, and `Copy package name` actions.
- Validate homepage URLs and allow only `http`/`https` opening.
- Load richer metadata on demand and cache it in memory.

### Acceptance criteria

- Homepage text is actionable rather than decorative.
- Opening a URL is always an explicit user action.
- List rendering does not issue one metadata request per package.

### Delivered

- Extracted a shared Package Inspector used by Installed, Discover, and Updates.
- Package rows on all three pages can select a detail view without changing checkbox selection.
- Inspector supports Copy Package Name; metadata-rich pages also support Copy URL.
- Homepage URL text itself is a clickable link and `Open Homepage` remains available as an explicit action.
- URL opening accepts only `http`/`https`, rejects credentials and unsafe schemes, never invokes a shell, and reports opener errors inline.
- URL opening uses `gio open` with `xdg-open` fallback.
- Updates displays current and available versions without issuing extra metadata requests.
- Removed rounded source/version badge backgrounds in favor of flat metadata text.
- Added URL validation tests; the full workspace passes 75 tests (13 UI + 62 core).

## Phase 8 — Keyboard-first desktop UX

**Status:** Completed (2026-07-28)

### Scope

- Add application-level keyboard event handling.
- Add focus IDs for search inputs.
- Initial shortcuts:
  - `Ctrl+K`: command palette/global package search;
  - `Ctrl+R`: refresh current page;
  - `Alt+1`–`Alt+4`: navigate pages;
  - `/`: focus page search;
  - `Esc`: dismiss confirmation/details/activity;
  - `Ctrl+Enter`: prepare primary action, with confirmation where required.
- Add visible focus styles for all interactive components.

### Acceptance criteria

- Shortcuts do not interfere with normal text editing.
- Every primary workflow can be completed with a keyboard.
- Focus returns to the initiating control when a transient surface closes.

### Delivered

- Added application-level shortcut capture with global shortcuts handled before text inputs and contextual shortcuts handled only after unconsumed events.
- Added `Ctrl+K` global package search, current-page `Ctrl+R`, `Alt+1`–`Alt+4` navigation, `/` search focus, `Esc` dismissal, and `Ctrl+Enter` primary actions.
- `Ctrl+K` switches to Discover and focuses the search field while preserving the user's existing source scope and selecting the current query text.
- Added stable search input IDs and focus restoration after confirmation, inspector, or Activity surfaces close.
- Added keyboard package traversal with Up/Down, Space selection toggling, `Ctrl+A` visible selection, and Tab/Shift+Tab focus traversal.
- `Ctrl+Enter` prepares or confirms install, update, removal, and Settings save actions while existing overlap/confirmation guards remain authoritative.
- Added visible shortcut guidance and stronger focus/hover outlines across shared interactive styles.
- Added focused shortcut-routing and bounded package-navigation tests; all 17 UI tests pass.

## Phase 9 — Responsive layout and themes

**Status:** Completed (2026-07-28)

### Scope

- Add wide, medium, and narrow desktop breakpoints.
- Make the sidebar collapsible at medium widths.
- Turn the persistent inspector into a drawer when space is limited.
- Stack toolbar controls at narrow widths.
- Add System, Light, Dark, and High Contrast preferences.
- Define complete semantic token sets for each appearance.

### Acceptance criteria

- The app remains usable in common tiling-window layouts.
- No custom component uses a light-only hard-coded color.
- Focus, disabled, error, warning, and selected states meet contrast expectations.

### Delivered

- Added explicit wide, medium, and narrow breakpoints driven by native window resize events.
- Wide layouts retain the full sidebar; medium layouts use a fixed compact icon navigation; narrow layouts provide a contextual Menu action and an in-sidebar Close action, plus a Show/Hide Details control when an inspector selection exists.
- Reduced the minimum supported window size to 640×520 and added scroll fallback for constrained content.
- Added persisted System, Light, Dark, and High Contrast appearance preferences plus live system-theme change handling.
- Added configurable native notification preference in the same isolated Settings draft/save workflow.
- Shared containers, inputs, surfaces, scrollbars, controls, status regions, and text now resolve semantic colors from the active Light, Dark, or High Contrast theme without light-only component styling.
- Added explicit high-contrast surface, divider, focus, disabled, success, warning, and error treatments.

## Phase 10 — Notifications, cancellation, and Activity Center

**Status:** Completed (2026-07-28)

### Scope

- Send configurable native completion/failure notifications.
- Introduce operation IDs and cooperative cancellation.
- Avoid unsafe process termination during system package transactions.
- Persist structured operation history with retention and clear-history controls.
- Redact sensitive command output and paths by default.

### Acceptance criteria

- Notification failure never affects package operations.
- Cancellation accurately reports completed and remaining work.
- History records are versioned, bounded, and privacy-conscious.

### Delivered

- Added an opt-in native notification preference; notifications run off the UI thread and failures are logged without changing operation outcomes.
- Added abortable operation tasks plus a cancellation token checked between manager transaction groups and a guarded `Cancel Task` control; cancellation never sends process-kill signals to package-manager commands already in flight.
- Added monotonic operation record IDs and a versioned, persisted structured Activity Center with newest-first records, package/source progress, failed source context, Clear, and Close controls.
- Activity history is bounded to 50 entries, trims long details, and redacts absolute path and credential-shaped tokens before display.
- Added focused tests for history retention and redaction; the full workspace passes 81 tests (19 UI + 62 core), strict workspace Clippy passes with warnings denied, and formatting/compilation/diff checks pass.

## Final verification

**Completed:** 2026-07-28

- `cargo fmt --all -- --check` passes.
- `cargo test --workspace --all-targets --no-fail-fast` passes all 81 tests (19 UI + 62 core).
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- `cargo check --workspace --all-targets` and `git diff --check` pass.
- Cargo reports future-incompatibility notices from upstream Iced/WGPU dependencies; there are no project warnings or test failures.

## Verification checklist

Run after every phase:

```bash
cargo fmt --all -- --check
cargo check -p updater
git diff --check
```

Add focused unit tests for state transitions and reload policy as those abstractions are introduced. Run the real application for changes involving layout, input, focus, themes, or window behavior.
