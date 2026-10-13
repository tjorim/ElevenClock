# ElevenClock Rust rewrite plan

Prepared 2026-10-13. Planning only; no Rust implementation has started.

This overview is supplemented by [the source dissection and implementation
contracts](RUST_REWRITE_DISSECTION.md), which define the concrete module layout,
Windows-specific design decisions, phase gates and issue breakdown.
Use that document for implementation details if the overview is less specific.

## Goal

Build a small, native Windows taskbar clock whose primary features are reliable
visual replacement of the system clock and user-defined date/time formats.
The motivating example is `2642.2`: 2026, week 42, Tuesday (Monday = 1).
Rewrite the retained functionality completely in Rust; feature parity with the
Python application is explicitly not the goal.

## Windows app experience

Install and launch from the Start menu. On first launch, show a small native
Settings window with a working clock preview and login startup off by default.
Subsequent launches focus the existing Settings window. Closing Settings keeps
the clock running; Exit in the clock/tray menu stops replacement and reveals
the native clock.

Users choose a date preset or edit its format in Settings, see immediate sample
output, select primary/all displays and appearance, and click Apply. No terminal,
configuration-file editing, environment variables or separate tools are needed.
A Support section explains unsupported shell states and offers Copy diagnostics.
The installer provides the normal end-user delivery path; ZIP builds are useful
for development and optional installation-free use.

The architecture is chosen for this Windows app. Prior Rust projects do not
establish a preferred stack. Settings storage is an internal implementation
detail, with all supported preferences available through the UI.

## Repository and evidence

- Upstream: https://github.com/marticliment/ElevenClock (archived when inspected).
- Inspected commit: `f967218e95b6f3c15e26f840e6090a6720528e39`.
- Fork: https://github.com/tjorim/ElevenClock, created by the user.
- Local checkout: `/workspace/ElevenClock`, branch `rust-rewrite-plan`, with
  the fork as `origin` and the archived source as `upstream`.
- `requirements.txt` uses PySide6, PyInstaller and Windows/Python helpers.
- `elevenclock/__init__.py:507` defines the Qt clock. Around lines 572–774 it
  enumerates `Shell_TrayWnd` / `Shell_SecondaryTrayWnd`, calculates placement,
  and creates cover windows. Around lines 1195–1336 it polls visibility and
  renders formatted time. Around line 799 its default click action is Win+N.
- `elevenclock/tools.py:166` combines Python strftime with arithmetic/time-offset
  expressions. The rewrite will retain plain formatting, not this expression DSL.
- README says Apache 2.0, but the checked-in LICENSE contains GPL v3. Preserve
  the LICENSE and attribution; resolve conflicting notices before distribution.
  A language rewrite does not by itself justify changing the fork's license.

## Proposed release scope

These are planning defaults, open to adjustment before implementation.

| Area | v1 commitment | Deferred or excluded |
| --- | --- | --- |
| Platform | Stock Windows 11, x64; publish exact tested builds | Windows 10, ARM64, Insider builds, ExplorerPatcher/StartAllBack, vertical taskbars |
| Placement | Bottom taskbar; primary or all monitors; one clock per taskbar | Arbitrary desktop placement, multiple clocks per monitor |
| Display | One or two lines, configurable format, font size, light/dark/high-contrast colors | HTML, animations, gradients, pixel-perfect translucent taskbar matching |
| Formatting | Local system time, ISO week date preset, ordinary date/time tokens, preview | Arithmetic expressions, offsets, extra time zones, internet time service |
| Shell behavior | DPI changes, auto-hide, fullscreen hiding, Explorer restart recovery | Reproducing notification counts, focus-assist indicators, Copilot controls |
| Interaction | Native calendar/notifications access, context menu, tray icon, settings, exit | User scripts, arbitrary commands, configurable gestures |
| Preferences | Format, monitor selection, font size, theme, opt-in login startup | Import of every legacy preference, full localization |
| Delivery | Per-user installer and Start menu entry; optional ZIP after validation | In-app updater, telemetry, store integration |

The operating system remains the time authority. The app neither changes the
system time nor changes global Windows date-format settings.

## Installation and privilege requirements

The supported end-user installation is **per-user and requires no administrator
rights**. A system-wide install is out of scope for v1.

- Default executable location: `%LOCALAPPDATA%\\Programs\\ElevenClockRust`.
- Private preferences/logs: `%LOCALAPPDATA%\\ElevenClockRust`, separate from binaries.
- Start menu shortcut: current user's Programs folder, resolved through Windows Known Folders.
- Installer/uninstaller: ordinary-user execution with no UAC elevation prompt.
  For Inno Setup use `PrivilegesRequired=lowest` and no elevation override;
  the app manifest requests `asInvoker`.
- Opt-in login startup: app-owned HKCU Run value, never HKLM, a service or an
  elevated scheduled task.
- Optional ZIP: extract and run from a user-writable folder without installation;
  it uses the same private per-user preferences and requires no elevation.
- Verify install, first launch, taskbar replacement, settings, startup, update,
  uninstall and ZIP execution using a standard non-administrator account on
  clean supported Windows builds, including missing-runtime cases.
- A runtime prerequisite must not silently trigger an elevated installer.
  Prefer compatible app-local deployment or supported linking choices. If any
  dependency or shell operation genuinely requires administrator rights, stop
  that design change and flag the reason and alternatives to the project owner
  before adopting it.
- Enterprise application-control policy can still block execution. Report this
  in the UI/documentation; requesting elevation is not the default workaround.

## Date/time contract

Use `chrono` with a documented, validated subset of its strftime syntax. Parse
once when settings change, keep the compiled token representation, and render
from a single local-time snapshot so two lines cannot disagree at midnight.

Recommended preset: `%g%V.%u`. `%g` is the two-digit **ISO week-year**, `%V` the
zero-padded ISO week (01–53), and `%u` the ISO weekday (Monday = 1, Sunday = 7).
Also offer `%y%V.%u` for users who explicitly want the **calendar year** paired
with ISO week numbers. The original example does not distinguish these choices.

| Local date | ISO preset `%g%V.%u` | Calendar-year variant `%y%V.%u` |
| --- | --- | --- |
| 2026-10-13, Tuesday | `2642.2` | `2642.2` |
| 2027-01-01, Friday | `2653.5` | `2753.5` |
| 2027-01-04, Monday | `2701.1` | `2701.1` |
| 2025-12-29, Monday | `2601.1` | `2501.1` |

These examples were checked using Python's independent `date.isocalendar()`;
the Rust implementation must reproduce them in unit tests. Do not silently use
`%W`, `%U`, Sunday = 0, or calendar-year `%y` for the ISO preset.

Initial supported tokens: `%Y`, `%y`, `%G`, `%g`, `%m`, `%d`, `%H`, `%I`, `%M`,
`%S`, `%p`, `%V`, `%u`, `%a`, `%A`, `%b`, `%B`, `%%`, plus literal Unicode and
one newline. Names are English initially. Reject unknown directives with a
specific error and retain the last valid configuration. Bound input length
(e.g. 256 characters) and output size; reject more than two display lines.

Default display: first line `%H:%M`, second line `%g%V.%u`. A single-line
`%g%V.%u` preset is equally supported. Preview includes today and a year-boundary
example. Measure rendered text against the clock slot and show a settings error
for overflow; do not silently obscure adjacent tray icons or truncate dates.

## Native architecture

Use a Cargo workspace under `rust/` during development. Keep the legacy app
available as reference until the Rust replacement passes the release gates.

```text
rust/
  crates/clock-core/       # format parsing/rendering, preferences, scheduling
  crates/clock-windows/    # taskbar discovery, geometry, events, rendering
  crates/clock-app/        # GUI startup and application orchestration; no Win32 FFI
```

- `clock-core`: platform-independent formatting, typed preferences and scheduling
  with injectable time. Use `chrono` for ISO calendar arithmetic and typed errors.
- `clock-windows`: Microsoft `windows` bindings, DirectWrite/Direct2D clock
  rendering, native Win32 settings controls and Windows lifecycle integration.
  Keep HWND, COM, hooks and graphics resources behind safe RAII wrappers.
- `clock-app`: GUI-subsystem executable and application coordinator, using the
  safe Windows facade. No public command interface, console attachment or
  command parser. Developer probes/tests are separate development artifacts.
- `PreferencesStore` persists validated per-user settings in an app-owned
  LocalAppData location. Its file encoding is selected and documented in R05
  based on reliable saves/migrations; it is not a public configuration format.
  Failed saves preserve the previous settings. Do not touch legacy user data.
- Use maintained stable Rust with a recorded MSRV, Cargo.lock and focused CI.
  Select dependencies for these responsibilities, not for similarity to another
  repository. Structured errors and bounded logs support the GUI.
- Event-driven updates for shell/display/theme/time/power/session changes.
  Use an adaptive wall-clock timer: one second when seconds are shown, at most
  one minute otherwise. Re-read local time at each wake and after resume or time
  changes. Only repaint changed content. Low-frequency shell recovery polling
  may complement events if necessary; avoid a permanent high-frequency loop.

## Taskbar integration: first and highest-risk milestone

Treat replacement as an external overlay. There is no assumed stable public API
for replacing Explorer's built-in clock contents. Do not inject into Explorer,
patch its memory, or permanently hide/delete its clock.

1. Enumerate primary and secondary taskbar windows and map each HWND to its
   monitor. Treat window class names and Explorer's UI structure as private
   compatibility details isolated behind `TaskbarBackend`.
2. Prototype native clock bounds discovery using UI Automation and available
   child-window geometry. Avoid localized label matching. Cache results and
   re-discover on relevant events; run potentially blocking UIA work outside
   the UI loop. Do not assume a clock child HWND exists on every Windows build.
3. Create one borderless, non-activating tool window covering only the clock
   region, with an opaque theme-matched background. The native clock must not
   show through. Preserve the notification bell and Show Desktop hit targets.
   Arbitrary text width cannot be solved by stealing neighboring tray space.
4. Use per-monitor DPI awareness v2 from process startup; keep coordinates
   consistently physical at the Win32 boundary. Recalculate on DPI, taskbar,
   display topology, resolution and primary-monitor changes.
5. Hide when the corresponding taskbar hides or a fullscreen app occupies that
   monitor. Keep z-order aligned with the taskbar; do not remain globally
   topmost over unrelated apps. Verify actual auto-hide animation behavior.
6. Detect Explorer restart via `TaskbarCreated` and stale-handle recovery.
   Recreate tray presence and clock overlays without restarting the app.
7. Prototype calendar activation via UIA invocation of the underlying clock.
   If unavailable, validate an explicit Win+N fallback on supported builds;
   do not assume synthetic input or cross-process mouse forwarding always works.
   Standard non-activating display behavior must not steal keyboard focus.
8. If bounds or build compatibility are uncertain, remove the overlay and leave
   the original clock usable. Show the reason through the tray/settings UI.
   On process exit or crash, the native clock reappears because it was never
   disabled. Test forced termination explicitly.

**Go/no-go gate:** demonstrate stable placement, native-clock coverage, usable
calendar access, unaffected adjacent hit targets, auto-hide, and Explorer
restart on real Windows before investing in settings polish. If neither UIA
nor geometry produces trustworthy bounds, narrow the advertised build support
or revisit the integration design. Do not label an approximate floating clock
as a finished taskbar replacement.

## Implementation sequence and completion criteria

The numbered issue dependencies in [the index](RUST_REWRITE_ISSUES.md) govern
assignment. The early foundation includes a native Settings window shell so
window behavior and controls can be assessed before the complete settings UI.


1. **Repository preparation.** Hosted fork created; upstream commit and
   licensing discrepancy recorded above. Add Cargo workspace,
   Windows manifest and CI on an implementation branch. No legacy deletion yet.
2. **Compatibility spike.** Implement the integration prototype above on one
   Windows 11 build, then a second build and mixed-DPI dual monitors. Record
   OS build IDs, discovery strategy, limitations and screenshots. Exit only
   after the go/no-go gate is met.
3. **Format engine.** Implement validated tokens, the week-date preset, preview
   and adaptive scheduling. Complete date-boundary and invalid-format tests.
   This can progress independently of shell discovery once implementation starts.
4. **Usable vertical slice.** Show `%H:%M` / `%g%V.%u` over the primary clock;
   add context menu, tray exit, stored preferences and last-valid fallback. Verify
   `2026-10-13 -> 2642.2` using an injected test clock.
5. **Reliability and monitor support.** Add all-monitor mode, mixed DPI, hotplug,
   theme/high contrast, fullscreen, auto-hide, lock/unlock, suspend/resume and
   Explorer recovery. Pass the manual matrix below before calling this a beta.
6. **Settings and delivery.** Add native format preview/settings and opt-in
   per-user startup and Copy diagnostics. Validate development ZIP builds, then
   ship a per-user installer with a Start menu entry and clean uninstall. Publish checksums, license notices and matching source;
   signing depends on availability of a signing identity.
7. **Rust cutover.** After acceptance, make Rust the default build, replace
   legacy build/release workflows, and remove Python/Qt runtime and packaging
   from the release tree. Preserve historical source in Git and an identified
   reference tag. Archive obsolete docs/assets selectively and write a concise
   migration guide documenting unsupported preferences.

## Validation and release gates

Automate `cargo fmt --check`, Clippy, core tests on Linux/Windows, and an MSVC
Windows release build in CI. Windows build success is not evidence of correct
Explorer behavior; use an interactive VM or physical desktop for shell tests.

Core cases: ISO year rollover, week 53, leap day, Monday/Sunday numbering,
zero padding, escaped percent, Unicode, unknown tokens, oversized input,
multiline limits, invalid configuration and single-snapshot rendering. Inject
time changes and DST repeated/skipped hours; never derive dates by adding a
fixed 24-hour duration. Test scheduling and recovery decisions deterministically.

Manual Windows matrix:

- Record two tested stock Windows 11 build numbers; x64, standard-user account.
- One and two monitors, negative monitor coordinates, primary monitor swap,
  hotplug and 100/125/150/200% DPI including mixed-DPI combinations.
- Taskbar auto-hide on/off, centered/left app alignment, fullscreen on one
  monitor, calendar flyout, bell and Show Desktop clicks.
- Light, dark and high contrast; long formats and font-size changes; keyboard
  access to settings and screen-reader labeling of the clock.
- Explorer kill/restart, app kill/restart, duplicate launch, sleep/resume,
  lock/unlock, timezone change, manual system-time change and date rollover.
- Start menu launch, first-run Settings, second-launch focus, close-to-background,
  Apply/Cancel/Reset, Copy diagnostics and Exit all work without a terminal.
- Startup/uninstall leave the native clock and system preferences intact.

Proposed performance budgets, to measure rather than claim: idle average under
0.2% total CPU and private working set under 40 MiB on a recorded reference
machine with two monitors. Measure for ten minutes in date-only, minute and
seconds modes. Run an eight-hour soak; no accumulating windows, hooks, handles
or memory. Reconsider budgets based on measured native rendering costs.

## Decisions to settle before release

- Confirm ISO week-year as the default, or select calendar-year `%y` explicitly.
- Record the Windows builds actually validated by the compatibility spike.
- Confirm the trimmed feature list, naming and visual defaults.
- Resolve README/LICENSE inconsistency and retain applicable asset notices.

The immediate next engineering task is the Windows taskbar integration spike,
not a port of the existing settings UI.
