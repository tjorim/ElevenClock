# Rust rewrite: source dissection and implementation contracts

Prepared 2026-10-13. This is an implementation proposal and assignment contract,
not a claim that Rust code or Windows compatibility has already been validated.
The [overview](RUST_REWRITE_PLAN.md) contains the retained feature scope and date
examples. The [issue index](RUST_REWRITE_ISSUES.md) records the execution backlog.

## 1. Evidence and conclusions

ElevenClock source baseline:
[`f967218`](https://github.com/tjorim/ElevenClock/tree/f967218e95b6f3c15e26f840e6090a6720528e39).
All source locations below refer to this immutable revision. Technology choices
are justified by the Windows product requirements.

ElevenClock's six top-level Python modules total 7,893 lines: `__init__.py`
2,037, `settings.py` 3,463, `welcome.py` 1,279, `tools.py` 980, `globals.py`
132, and `versions.py` 2. This excludes external helpers, resources and
translations. The settings and onboarding layers alone are 4,742 lines.
These counts describe scope, not a target line-count reduction or speed claim.

The current app already implements visual replacement through overlay/cover
windows. Rust changes ownership, rendering, configuration and event handling;
it does not make Explorer's private interfaces stable. The decisive risk remains
finding and tracking the exact native clock rectangle without covering the
notification bell or Show Desktop region.

### Current runtime flow

```mermaid
flowchart TD
  Boot[Python entry point and mutable globals] --> Load[loadClocks]
  Load --> Clock[Qt Clock per display]
  Clock --> Cover[Optional cover Clock and helper widgets]
  Clock --> Text[Text update thread]
  Clock --> Vis[Visibility and background polling thread]
  Clock --> Time[Internet time thread]
  Boot --> Other[Updater, monitor, instance and notification workers]
  Settings[Qt settings and preference files] --> Restart[restartClocks]
  Other --> Restart
  Restart --> Load
```

The responsibilities overlap: a Clock discovers Explorer, positions itself,
formats time, samples background pixels, handles gestures, tracks notifications,
and manages threads. Changes to a preference can rebuild the clocks. The rewrite
should preserve the visible behavior we need while separating those operations.

## 2. Windows product and technical decisions

The product is a native Windows app: install, launch, configure, use and diagnose
through Windows UI. A user's earlier Rust project is not a stack requirement.

### User experience contract

- Per-user installation creates a Start menu shortcut with the app icon.
- First launch shows Settings with live preview; login startup is initially off.
  Persist first-run completion internally; login startup does not reopen Settings.
- A later Start menu launch opens/focuses Settings in the running instance.
- Closing Settings leaves the clock running. The fixed clock/tray menu offers
  Settings, Pause/Resume replacement and Exit. Exit reveals the native clock.
- Settings groups: Date & time (presets, format editor, live preview and token
  help); Appearance (primary/all displays, font size, system/light/dark);
  Startup (opt-in checkbox); Support (version, status, Copy diagnostics).
- Apply validates, saves and updates without restarting. Cancel discards drafts.
  Reset to defaults stages a draft; Apply commits it. Errors stay beside the
  relevant control; save errors preserve previous settings and editable drafts.
- The window uses standard focus, keyboard navigation, accessible labels, DPI
  scaling, taskbar/icon behavior and tested theme/high-contrast treatment.
  Do not assume a Win32 default dialog automatically meets those expectations.
- No terminal, path override, environment variable, hand-edited file or developer
  probe is part of normal installation, configuration, support or recovery.

| Requirement | Proposed implementation | Why / validation |
| --- | --- | --- |
| Precise Explorer integration and native controls | Microsoft `windows` crate; Win32 window/control APIs | Direct access to shell, DPI, focus, accessibility and lifetime semantics |
| Readable taskbar text | DirectWrite + Direct2D | Measure exact text fit, honor DPI and recover graphics resources |
| Small native Settings window | Standard Win32 controls behind safe Rust wrappers | R01 includes a window/control prototype; validate keyboard, DPI and themes before completing R10 |
| Correct ISO dates and previews | `chrono` with a validated token subset | Deterministic calendar arithmetic tested at year boundaries |
| Preferences invisible to users | Typed `Preferences` and private `PreferencesStore` | R05 picks an internal encoding for an app-owned LocalAppData file; no user-facing format/API |
| Safe Windows boundary | Pure core + Windows adapter + GUI coordinator | Core is testable without Explorer; unsafe code is isolated |
| Responsive UI | Windows message loop with bounded discovery work | Blocking UI Automation never runs in window callbacks |
| Understandable failures | Typed errors; optional `thiserror` and contextual app errors | Convert technical failures into inline Settings messages/status |
| Support without a terminal | In-app status, Copy diagnostics, bounded local logging | A plain-text report can be inspected and copied deliberately |
| Reproducible releases | Stable Rust, documented MSRV, locked build and Windows CI | R01 records a tested toolchain compatible with chosen dependencies |
| Efficient native app | Minimal dependencies and measured event-driven work | No browser runtime, command parser or async runtime by default |

Rust 2024 is suitable for a new codebase; the MSRV must be selected and verified
for this project in R01, not copied from another app. Select optional error/log/
serialization crates when their owning task needs them. A serializer is allowed
as an internal detail but does not make a configuration-file interface part of
the product. No `clap` dependency or public command suite is planned.

Native Win32 controls are the initial choice, not a claim of finished Windows 11
visual polish. The R01 UI prototype must prove layout, focus, scaling and theming;
if it cannot meet the UI contract with documented APIs and reasonable complexity,
record a bounded native-UI alternative before R10 rather than shipping a console
fallback or an unstyled debug panel. Taskbar rendering remains a separate native
surface regardless of the eventual Settings control implementation.

## 3. Before → after implementation map

Paths on the right are beneath `rust/crates/`. R-items refer to the issue index.

| Current source / mechanism | Concrete replacement | Observable change / owner |
| --- | --- | --- |
| `__init__.py:1–87,1894–1989`, `globals.py`: imports, shared objects, boot | `clock-app/src/app.rs`, `clock-windows/src/runtime.rs`; explicit AppState and RAII resources | One instance and one UI event loop; no mutable Python-style globals. R01/R08 |
| `__init__.py:179 loadClocks`, `:276 restartClocks` | Reconcile overlays by transient taskbar ID and shell generation | Update/recreate only affected windows; avoid restarting all clocks for a format change. R06/R08 |
| `__init__.py:572`, `:700–774`: taskbar class lookup, preferred dimensions, covers | `clock-windows/src/taskbar/{probe,uia,geometry}.rs`; measured clock-only bounds | No guessed fixed-width slot. Unknown geometry leaves native clock visible. R02/R03 |
| `__init__.py:624–640,1194–1297`: topmost flags, registry-byte auto-hide, mouse polling | `clock-windows/src/events.rs` + core visibility reducer | Events plus bounded recovery polling; hide in step with actual taskbar/fullscreen state. R06/R08 |
| `__init__.py:988 loadTimeFormat`, `tools.py:153–190` expression DSL | `clock-core/src/format.rs`, explicit token allowlist | ISO preset and errors with positions; no arithmetic or offset expressions. R04 |
| `__init__.py:1314 updateTextLoop` formats every 0.1/0.25 seconds | `clock-core/src/schedule.rs`, Windows timer and one local snapshot per update | Second or minute cadence; repaint only when text/geometry/theme changes. R06/R08 |
| `__init__.py:1120 checkAndUpdateBackground`: grab a screen pixel and infer contrast | `clock-windows/src/{render,theme}.rs`, system theme/high-contrast colors | Opaque readable coverage; no screenshot sampling or promised translucent match. R07 |
| `__init__.py:799–841,1340–1407,1850`: keyboard shortcuts and gesture actions | `clock-windows/src/{interaction,tray}.rs`; native clock invocation with validated fallback | Click opens calendar/notifications; right-click fixed app menu. No arbitrary commands. R03/R09 |
| `tools.py:205–278`: per-setting files in ~/.elevenclock; restart side effects | `clock-core/src/preferences.rs` + `clock-windows/src/preferences_store.rs` | Private per-user persistence; UI validates and commits immutable preferences. R05 |
| `settings.py:43 SettingsUI`, `:3361 CustomSettings`; `welcome.py` | `clock-windows/src/settings.rs`, core preview and validation | One compact settings dialog, no wizard or per-monitor override UI. R10 |
| `__init__.py:285`: timestamp lockfiles scanned every two seconds | `clock-windows/src/instance.rs` session/user-scoped named mutex and fixed settings signal | Duplicate launch opens existing settings; no heartbeat files. R08 |
| `__init__.py:245,340,1943`: monitor/sleep polling and process restart | Display, DPI, power, session, time and TaskbarCreated messages | Resume/re-discover in process; dispose stale HWNDs. R08 |
| `__init__.py:1474 loadInternetTimeOffset`, `external/timezones.py` | OS local time only | No internet requests, custom offset or timezone catalog. R04/R08 |
| `__init__.py:238`, `external/WnfReader.py`, label notification helpers | Omit; preserve Explorer's adjacent notification controls | No undocumented WNF subscription or duplicated bell. R03/R09 |
| `__init__.py:88–168` updater, settings announcements, language downloads | Omit from runtime; manual releases | No updater/announcement/download threads. R14/R15 |
| `tools.py:113`, startup stdout capture, crash browser URL | `clock-app/src/{diagnostics,logging}.rs` with tracing | In-app status and Copy diagnostics with capped local logs; no automatic upload. R11 |
| `ElevenClock.iss:130` autostart; existing AppId and reset/uninstall tasks | New app identity, own startup entry and dedicated per-user install directory | Opt-in startup; uninstall does not kill/remove legacy ElevenClock. R12/R14 |
| `build.cmd`, `elevenclock.spec`, Python compileall CI | Cargo locked MSVC build, core tests, native artifact smoke tests | No Python/Qt needed in final release; legacy removal only after gates. R01/R14/R15 |

## 4. Target boundaries and interfaces

```mermaid
flowchart LR
  App[GUI startup / app coordinator] --> Core[clock-core: format, preferences, schedule, state]
  App --> Win[clock-windows: safe facade]
  Win --> Core
  Shell[Explorer / Windows events] --> Win
  Win --> UI[Overlay, settings, tray, UIA]
  Settings[Typed preferences from native Settings] --> Core
```

Dependency direction: app → windows → core, and app → core. Core must never
import Win32 types. `#![forbid(unsafe_code)]` in core and app. Native controls,
message-loop callbacks, clipboard, startup and filesystem FFI all live
in the Windows crate behind safe wrappers. Use RAII and document thread affinity
and callback lifetime. HWNDs and COM interfaces never cross worker channels.

```text
rust/Cargo.toml, Cargo.lock
rust/crates/clock-core/src/{lib,error,model,format,preferences,schedule,state}.rs
rust/crates/clock-windows/src/{lib,runtime,events,instance,preferences_store}.rs
rust/crates/clock-windows/src/taskbar/{mod,probe,uia,geometry}.rs
rust/crates/clock-windows/src/{overlay,render,theme,interaction,tray,settings,support,startup}.rs
rust/crates/clock-app/src/{main,app,diagnostics,logging}.rs
docs/windows-evidence/                 # exact build IDs and manual outcomes
```

R01 defines and documents the minimal contracts; subsequent issues add behavior
without broad API redesign. Proposed shapes, not pre-validated Rust source:

```rust
CompiledFormat::parse(&str) -> Result<CompiledFormat, FormatError>
CompiledFormat::render(DateTime<FixedOffset>) -> Result<ClockText, FormatError>
CompiledFormat::cadence() -> Cadence // Second or Minute, date-only is Minute
Preferences::validate(self) -> Result<ValidatedPreferences, PreferencesError>
PreferencesStore::load() -> Result<ValidatedPreferences, StoreError>
PreferencesStore::save(&ValidatedPreferences) -> Result<(), StoreError>
next_delay(snapshot, cadence) -> Duration
reduce(state, event) -> (state, Vec<Effect>)
WindowsRuntime::run(validated_preferences) -> Result<(), PlatformError>
```

Shared models: `PhysicalRect` uses signed coordinates and validated positive
size; `TaskbarId` is transient and paired with a `ShellGeneration`; a
`TaskbarSnapshot` contains taskbar rect, optional proven clock rect, monitor
identity, DPI, primary flag, visibility and discovery strategy. Selected
monitors are simply `primary` or `all`, so v1 needs no persistent display IDs.

Core state per taskbar: `Unavailable`, `Hidden(reason)` or `Visible`. Inputs
include `TopologyChanged`, `DiscoveryResult(generation, snapshots)`,
`VisibilityChanged`, `Tick`, `TimeChanged`, `ThemeChanged`, `PreferencesApplied`,
`ExplorerLost`, `SessionLocked`, `SessionUnlocked` and `Quit`. Effects include
`Discover`, `Create/DestroyOverlay`, `SetBounds`, `Paint`, `Hide` and `ArmTimer`.
Discard responses from older shell generations. Failures are explicit, never
silently converted to a plausible rectangle.

### Time and format contract

Keep the overview's token allowlist. Limit input to 256 Unicode scalar values,
at most two nonempty lines, and rendered output to 512 Unicode scalar values. Normalize
CRLF once; permit newline but reject other control characters. No `%n`, padding
modifiers, unlisted directives, HTML or arithmetic. Parse into owned tokens once.
Report an offending token with a documented character-index span.

Use `%H:%M\n%g%V.%u` as the proposed initial preset; the settings UI must call it
"ISO week date". Offer a "Calendar year + ISO week" preset with `%y%V.%u`.
The user example is compatible with both; do not hide their New Year difference.
Take one OS-local snapshot for all clocks and preview lines. Snapshot fixed
offset is a value at an instant, not a persistent timezone rule. Re-resolve
local time on each wake/resume/timezone change; never add 24 hours to find the
next local date. Align normal waits to the next second/minute, using a monotonic
wait and a fresh wall-clock read on wake. Date-only formats recheck each minute.

### Private preference persistence contract

The public interface is Settings, not a file schema. The core preference model
contains format, monitors (primary/all), font size (6–24 pt) and theme
(system/light/dark), with an internal version for migration. Initial values are
`%H:%M\n%g%V.%u`, primary display, 9 pt and system theme. Keep first-run state
internal. Read login-startup state from Windows rather than duplicating it.

R05 chooses and documents a maintained serialization approach for a single
app-owned preference file under LocalAppData, resolved through Known Folders.
The encoding and filename are private and can evolve with migrations. No
`--config`, environment-variable path override, manual-edit instructions,
exported schema or explicit Reload command. Tests use injected temporary stores.

A missing store gives defaults. Corrupt/incompatible data is preserved, with a
recoverable Settings message and safe defaults for that launch; do not silently
overwrite it. Save validates first, writes/flushes/closes a temporary file in the
same directory and uses a tested Windows replacement operation. Failed save
leaves disk and active preferences unchanged. Reject unsupported stored versions
without destructive downgrade. Reset to defaults is an explicit UI draft.

Validate layout on each selected active monitor before Apply; future text/DPI
overflow hides that monitor's overlay and explains the problem in Settings
instead of widening over tray controls. No watcher or legacy import in v1.

### Shell integration contract and experiments

Enumerate `Shell_TrayWnd` and `Shell_SecondaryTrayWnd`. Prototype UIA and native
child geometry; neither is assumed to expose a clock-only rectangle on every
Windows build. UIA provider latency and locale-independent selectors must be
measured. A rectangle is usable only if it belongs to the current Explorer
generation, fits the taskbar/monitor, and excludes neighboring controls.

The UI thread owns overlay HWNDs and graphics resources. One dedicated COM
worker owns UIA objects and sends value snapshots tagged with generation IDs.
Use a bounded/coalesced request queue and a result deadline. A deadline only
stops waiting: it cannot be assumed to cancel a blocking COM call. Keep at most
one stalled worker, do not respawn threads without bounds, and remove overlays
on failure. If evidence shows COM cannot provide bounded teardown, R02 must
select a cancellable alternative or an isolated same-executable helper process
and amend the ADR before implementation proceeds. Never invent cancellation.

Overlay is `WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE` with borderless style, DPI v2
manifest, and an opaque background drawn only inside the measured clock slot.
Prototype relative z-order with `SetWindowPos` and test activation/taskbar
transitions. Do not keep an unconditional always-topmost window over apps, parent
into Explorer, change Explorer styles, or permanently disable the original clock.

Use `WM_DISPLAYCHANGE`, `WM_DPICHANGED`, `WM_SETTINGCHANGE`, `WM_THEMECHANGED`,
`WM_TIMECHANGE`, power/session messages and registered `TaskbarCreated` where
applicable; supplement with filtered out-of-context WinEvent hooks and a slow
recovery check (proposed two seconds). Auto-hide tracking may need a short,
bounded poll during a transition. Measure this rather than promising pure event
coverage. Hooks and callbacks must not do blocking UIA or expensive rendering.

Click strategy: first prove UIA invocation of the underlying clock without
invoking the overlay itself. If unsupported, test one deliberate Win+N action
per click with no stuck modifiers or repeated input. Unknown/failed invocation
removes the overlay for native access and reports the limitation. No synthetic
mouse click relay. Keyboard activation of the tray opens the same menu; settings
has normal tab order. A minimal UIA provider for the overlay exposes the displayed
clock and an invoke action, without announcing every second.

### In-app support and developer tooling contract

Ship a Windows GUI-subsystem executable that never opens or attaches a console.
Normal users launch it through the Start menu, installer or executable. Private
activation/startup plumbing is allowed; it is not a documented command interface.

Settings → Support shows app version, Windows build, replacement status and an
actionable explanation if the native clock is being used as fallback. Display
an explicit Refresh status button backed by bounded discovery so Settings cannot
freeze. R11 consumes the runtime's safe snapshot API; no new diagnostic service
or public IPC protocol is required.

Copy diagnostics puts a human-readable report on the clipboard only when clicked:
app/build version, Windows build/architecture, discovery strategy, monitor
bounds/DPI, replacement status and error reasons. Show exactly what will be
copied in a selectable preview. Exclude usernames, file paths and unrelated
window titles. Announce success/failure accessibly; clipboard contention must
not crash or block the UI. No automatic upload or network logging.

Keep local logs bounded (1 MiB per file, at most three retained files) and log
state transitions instead of every repaint. Logging failure must not stop the
clock. No environment variable is required to use the Support UI. Internal
structured logging is an implementation choice for R11.

Format fixtures, taskbar probes and performance harnesses are development/test
artifacts under examples/tests/tools. They may accept arguments for reproducible
testing, but are excluded from the installed product and end-user support
instructions. Automated tests use core APIs and GUI harnesses instead of a
shipped `format`, `config validate` or `doctor` command.

The end-user release includes a per-user installer, Start menu shortcut and
uninstaller. Installation-free ZIP builds remain optional and use the same
private per-user preferences. Neither distribution requires a terminal.

## 5. Phases and assignment rules

| Phase | Items | Exit gate |
| --- | --- | --- |
| P0 Foundation | R01 | Workspace, contracts, CI and contributor commands agree |
| P1 Feasibility | R02 discovery, R03 overlay prototype | Clock-only coverage and native access demonstrated on exact Windows builds |
| P2 Pure logic | R04 formatter, R05 preferences, R06 scheduler/state | Deterministic contracts/tests; can progress alongside P1 after R01 |
| P3 Working shell | R07 renderer, R08 lifecycle, R09 interaction/accessibility | Multi-monitor clock survives shell/display changes and remains usable |
| P4 Usability | R10 settings, R11 diagnostics, R12 startup | Complete Windows UI flow: configure, diagnose, start and stop without a terminal |
| P5 Release | R13 acceptance, R14 packaging/licensing, R15 cutover | Measured Windows evidence, install/rollback checks, Rust is default |

The exact dependency graph and issue URLs live in the issue index. Dispatch
R04 and R06 independently after R01; R05 follows R04 so it can validate formats.
Keep shared contract changes serialized.
P3 is blocked on a passed P1 gate. Discovery and shell/COM work need a
Windows-capable agent and experienced review. Pure logic tasks are appropriate
for lower-level agents. This is a plan for later assignment, not a claim that
agents have run or that Windows evidence exists.

Each issue must contain baseline source locations, owned files, exact behavior,
non-goals, dependencies, test commands and handoff evidence. Agents branch from
the integration branch after prerequisites land, implement one issue per PR,
and do not rename shared types unilaterally. Initially use `rust-rewrite-plan`
for documentation; R01 creates `rust-rewrite` from main with these planning docs
included. Feature branches target `rust-rewrite`; no automatic merge to main.

If a required Windows desktop is unavailable, an agent may complete code and
unit tests but must leave the manual gate unchecked. Do not mark the issue done
based on cross-compilation, mocked shell data or screenshots from another build.

## 6. Validation and release criteria

CI: Linux core tests/doc tests; Windows MSRV check of workspace/all targets;
Windows stable fmt, Clippy with warnings denied, tests/doc tests and locked
x86_64-pc-windows-msvc release build. Add dependency audit/license checks and
artifact smoke tests. No interactive Explorer result is inferred from CI.

Fixtures include ISO week 53 and year boundaries, Monday/Sunday, leap day,
Europe/Amsterdam spring/fall DST snapshots with explicit offsets, manual clock
jumps, stale discovery generations, negative monitor coordinates, DPI changes,
unsupported preference versions and failed saves. Format fuzz/property tests must bound output
and never panic on Unicode or malformed tokens.

Manual evidence records exact Windows build/patch, hardware, monitors, DPI,
theme, commands, app commit and outcome. Exercise two supported stock Windows
11 x64 builds; one/two displays; 100/125/150/200% mixed DPI; auto-hide; fullscreen
per monitor; all adjacent clicks; calendar; screen reader; lock/resume;
hotplug/primary swap; Explorer restart; forced app kill; Start menu/first-run and
second-launch behavior; Apply/Cancel/Reset; diagnostic preview/copy; and clean
uninstall. A tester must complete the entire end-user journey without a terminal,
manual preference edits or developer tools.
The tested builds become the support list; this document does not choose build
numbers without running the experiment.

Performance targets remain provisional: under 0.2% total-machine idle CPU and
under 40 MiB private working set on a recorded two-monitor reference machine.
Use process CPU-time delta / (wall time × logical processors), not an ambiguous
Task Manager screenshot. Measure ten minutes in date/minute/seconds modes and
report memory, timer wakes and discovery rate; measure helper processes too if
R02 selects isolation. Eight-hour soak: sample handles/threads/memory for growth.
Document failures; do not weaken thresholds silently to obtain a pass.

Distribution retains applicable GPL/asset notices until the README/LICENSE
conflict is resolved from evidence. A Rust rewrite does not reset copyright.
Use a new installer AppId/startup identity. Archive the old baseline in Git,
retain legacy settings, and provide rollback instructions. Remove Python,
PyInstaller, Qt and obsolete workflows only in R15 after acceptance and packaging.

## 7. Installation privileges

Follow the [installation and privilege requirements](RUST_REWRITE_PLAN.md#installation-and-privilege-requirements): per-user installer and optional ZIP, standard-user execution, no elevated prerequisites, and a mandatory non-administrator acceptance run. R12 and R14 own startup and packaging verification respectively. Flag unavoidable administrator requirements to the project owner before adopting them.
