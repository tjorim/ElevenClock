# R08 event-driven runtime, multi-monitor reconciliation and Explorer recovery

Status: **runtime, lifecycle, supervision and tests implemented; the interactive and
Explorer-correlated gates below are unchecked.** The authoring environment was Linux;
Windows code was type-checked for `x86_64-pc-windows-msvc` only. **P1 remains
NO-GO** ([overlay evidence](../windows-evidence/overlay-spike.md), [#21](https://github.com/tjorim/ElevenClock/issues/21),
[#23](https://github.com/tjorim/ElevenClock/issues/23)), but the project is a work in
progress with no release, so there is **no support gate**: the R02 experimental
candidate always places the clock after geometric validation. A failed, late or stale
discovery still fails closed to an authoritative empty batch, which removes the overlays
and leaves the native clock untouched.

## What changed

| Item | Where | Contract |
| --- | --- | --- |
| `Runtime<P: Platform>` | `clock-windows/src/runtime.rs` | Single coordinator between platform inputs and R06's reducer. No second state machine. Pure of OS access; 30 deterministic tests with a scripted platform (plus shell, protocol, gate, candidate, event and time tests). |
| `NativePlatform`, `run` | `runtime/native.rs`, `runtime/overlay_window.rs` | One UI thread: hidden top-level host window, `SetTimer` one-shots, WinEvent hooks, session notifications, overlay windows, Direct2D (R07), Settings dialog. |
| `events` | `events.rs`, `events/native.rs` | Window message / WinEvent → `Signal`; coalescing `Pending`; bounded `TransitionPoll`. Callbacks only queue. |
| `instance` | `instance.rs`, `instance/native.rs` | `Local\ElevenClock.Instance.<SID>` mutex, `HOST_CLASS`/title lookup, `WM_SHOW_SETTINGS`, `Role`/`LaunchMode`. |
| `taskbar::{protocol,worker,helper,candidate}` | `taskbar/` | Wire contract, supervised worker, same-executable helper, experimental candidate selection. See [ADR 0002](../adr/0002-same-executable-discovery-helper.md). |
| `clock_time` | `clock_time.rs` | One fresh zone/DST-resolved `DateTime<FixedOffset>` per update. |
| App lifecycle | `clock-app/src/app.rs`, `settings.rs`, `settings.rc` | Modeless Settings; close keeps running; **Exit** button; second launch focuses. |

Shared contracts (`clock-core`) are **unchanged**. `TaskbarObservation` gained an
`hwnd` field and `probe::collect_with(generation, with_uia)` was added; the R02/R03
development binaries are otherwise untouched. The earlier
`unverified-overlays` Cargo feature and the support gate were removed. New
`windows` crate features only (no dependency or lockfile change).

## Event-to-core mapping

| Source | Signal | Runtime action → core event |
| --- | --- | --- |
| `WM_DISPLAYCHANGE`, `WM_DPICHANGED`, `WM_SETTINGCHANGE` ("TraySettings" or `SPI_SETWORKAREA`), WTS console/remote connect | `TopologyChanged` | 250 ms trailing settle, then `TopologyChanged` (one discovery per burst; one follow-up if topology changed while one was in flight) |
| `WM_THEMECHANGED`, `WM_SYSCOLORCHANGE`, `WM_SETTINGCHANGE` (`ImmersiveColorSet`, other) | `AppearanceMaybeChanged` | re-read system appearance + font; `ThemeChanged{revision}` only on a real change |
| `WM_TIMECHANGE` | `TimeChanged` | fresh local snapshot → `TimeChanged` |
| `WM_POWERBROADCAST` suspend / resume | `Suspended` / `Resumed` | resume: `TimeChanged`, shell check, appearance, topology |
| `WM_WTSSESSION_CHANGE` lock / unlock | `SessionLocked` / `SessionUnlocked` | `SessionLocked`; unlock: `SessionUnlocked` (reducer requests discovery) + fresh `TimeChanged` + shell check |
| registered `TaskbarCreated` | `TaskbarCreated` | forced shell re-identification → `ExplorerLost` (generation + 1, old overlays destroyed, new discovery) |
| WinEvent show/hide/location (Explorer PID only, `Shell_*TrayWnd` only) | `Shell(TaskbarChanged)` | start bounded transition polling; observe → `VisibilityChanged` / rediscover |
| WinEvent foreground / minimize-end / move-size-end (system-wide, low rate) | `Shell(ForegroundChanged)` | observe → `FullscreenChanged` per monitor |
| `WM_ENDSESSION`, `WM_CLOSE` on host, Settings **Exit** | `EndSession` / exit flag | `Quit` |
| `WM_TIMER` (clock one-shot) | — | one fresh snapshot → `Tick`; reducer returns `ArmTimer` |
| `WM_TIMER` (poll one-shot) | — | shell identity check, due retries / settle, observation pass |
| worker report | `Discovery(report)` | accept only the current generation *and* matching shell identity; failure → authoritative empty batch (fail closed) + backoff |

## Fallback polling rates and timers

* **Recovery poll: 2 s** (documented in the dissection). One `SetTimer` one-shot; cheap
  checks only: shell identity (PID + creation time), then one observation per taskbar.
* **Transition poll: 100 ms**, begun by a taskbar WinEvent, ended after two unchanged
  observations or **15 polls**, then back to 2 s.
* **Topology settle: 250 ms** trailing edge. **Observation-triggered rediscovery
  throttle: one per 2 s**, and a given odd rectangle triggers at most one.
* **Discovery retry backoff:** 2, 4, 8, 16, 32, then 60 s; any real topology event
  resets it. An empty batch while Explorer runs is retried the same way.
* The clock timer is a single monotonic one-shot aligned to the next local
  second/minute boundary; a re-armed poll timer is never postponed by clock ticks.
  There is no high-frequency loop.

## Shutdown and failure semantics

`Quit` destroys every overlay (reducer effects), cancels both timers, unhooks every
WinEvent hook, unregisters session notifications, terminates and reaps any helper,
joins the worker (bounded), destroys the host window and unregisters classes. A panic
in a window procedure or hook becomes a clean shutdown with an error box. A crash
removes all windows and (via the kill-on-close job) any helper. Explorer is never
modified, injected into or parented to, so the native clock is always usable.

## Lifecycle contract

Interactive launch (Start menu, double-click) opens Settings; login startup
(`--startup`, the exact marker R12 will register) runs quietly. A second launch
posts `WM_SHOW_SETTINGS` to the existing host window (lookup by class and
SID-derived title, bounded retry for simultaneous launches) after
`AllowSetForegroundWindow`, and exits 0 without creating a clock. Closing Settings
(Close, Escape, X, system menu) destroys only the dialog. **Exit ElevenClock** in
Settings, a polite `WM_CLOSE` to the host, or session end quits. A tray/clock menu
Exit belongs to R09.

## Verification performed (Linux authoring environment)

```sh
cargo +1.90.0 fmt --manifest-path rust/Cargo.toml --all --check
cargo +1.90.0 clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo +1.90.0 clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo +1.90.0 test  --manifest-path rust/Cargo.toml -p clock-core -p clock-windows --locked
```

The Windows type-check is not execution. Results are in the pull request description.

### What the tests prove (and do not)

* `runtime::tests` (scripted platform): start sequence; all/primary monitors with
  negative coordinates; clock/time/DST-style jumps use
  fresh snapshots; topology bursts and in-flight refresh; `TaskbarCreated`, PID reuse
  and missing Explorer; stale-generation and wrong-shell rejection; failure →
  fail-closed → bounded backoff → recovery; fatal teardown disables discovery;
  auto-hide, relocation (no loops), DPI/unplug, primary swap, per-monitor fullscreen,
  lock/unlock, appearance/font, live preference changes, overflow/render/placement
  reasons, quit idempotence, 50 consecutive Explorer replacements without overlay
  accumulation.
* `tests/discovery_worker.rs` (real child processes, every host; Windows adds the job
  object and a parent-loss test): current-generation publish, foreign generation,
  malformed/oversized/crashing helpers, **deadline discards a complete-looking partial
  report and the helper is dead before the result is published**, superseding
  generation kills the running helper and only the newest publishes, request bursts
  coalesce, shutdown kills and joins, and **60 mixed failures neither grow threads,
  handles (±2) or memory (+16 MiB) nor leave helpers or temporary files**.
  The helper is a *fixture process* (this test executable): it exercises supervision,
  not a UI Automation provider.
* `tests/uia_lifetime.rs` adds real UIA/provider lifetime and opt-in Explorer
  correlation checks; see the hosted evidence below. Overlay visibility, z-order
  and DPI require the separate desktop harnesses.

## Hosted results (PR #28, head 0835017)

[Lifecycle run 38581394740](https://github.com/tjorim/ElevenClock/actions/runs/38581394740)
passed on Server 2025 x64, Windows 11 ARM64 native and Windows 11 ARM64 emulated x64:
quiet start, second launch (one Settings window, duplicate exits 0), close-to-background,
three Explorer restarts with no helper accumulation and bounded thread/handle growth,
and Exit leaving no process. That run predates opening the overlays; its overlay
observation step (informational, Server only) used the since-removed
`unverified-overlays` build. The overlay counts are no longer a separate step: the
lifecycle run passes `-ObserveOverlays` and records them in the artifact, not as an
acceptance claim. The Rust foundation workflow (native Windows tests, including the
Windows job-object and parent-loss worker tests, and the updated Settings/UI smoke
scripts) passed on earlier heads. The R03 overlay workflow stays red for its known,
unrelated P1 recovery failure. The first lifecycle failure was a harness bug
(PowerShell passes `$null` to `FindWindowW` as an empty title), not an app defect.

## UIA lifetime evidence (hosted, Server 2025 x64)

`tests/uia_lifetime.rs` (custom harness; the CI step sets `ELEVENCLOCK_RS_UIA_EXPLORER=1`
and `ELEVENCLOCK_RS_EVIDENCE_DIR`) records raw per-call HRESULTs. Observed on the
lifecycle run for head 15cadfd:

| Case | Observed |
|---|---|
| Healthy controlled window | inventory completes, 11 calls, longest 50 ms |
| Provider whose UI thread never pumps | inventory hits the 4 s deadline (4.015 s), exactly one helper spawned and reaped |
| Provider killed with a client call outstanding | the call returned on its own (`S_OK`, no hang) |
| Elements retained across provider death | `0x80040201` (`UIA_E_ELEMENTNOTAVAILABLE`) for every later call |
| **Real Explorer** suspended (`NtSuspendProcess`) to hold a taskbar call in flight ≥ 500 ms, then `taskkill /F` | the outstanding call returned after the terminate (`0x8000FFFF`); all six retained-object calls failed `0x80040201`; shell identity changed (pid 8720 → 6140) |

Findings that shaped the design: a UIA call in flight when the shell dies is **not**
guaranteed to fail with a useful HRESULT (an earlier attempt returned `S_OK`), so the
parent never trusts a late or successful-looking helper result — it relies on the
generation/shell-identity check and the helper deadline. Retained elements always fail
`0x80040201`, so no element is cached across a generation. On the hosted Server image
Windows did **not** restart the shell after `taskkill`; the harness started `explorer.exe`
itself (`shell_started_manually: true`, no taskbar for 45 s), so this proves correlation
and stale-object behaviour, not Windows' own shell auto-restart timing.

### Issue #35 confirmation (2026-10-16)

The UIA lifetime step in [run 38839269796](https://github.com/tjorim/ElevenClock-rs/actions/runs/38839269796)
passed on Server 2025 x64 after #33's handle-accounting fix, as well as both
Windows 11 ARM64 targets (native and emulated x64). The
[retained evidence](../windows-evidence/uia-lifetime-hosted/README.md) records the
exact jobs, measurements and limits. [ADR 0002](../adr/0002-same-executable-discovery-helper.md#measured-com-behaviour-issue-35-2026-10-16)
now records the measured behaviour; the process deadline and teardown contract
are unchanged. Error-returning provider modes and geometry injection remain
optional extensions, as clarified in the issue comment.

## Postponed to a real Windows machine

By decision of the repository owner, the checks that need an actual (non-hosted)
Windows desktop are **postponed**; they remain unchecked below and are not claimed.
They belong in the shared desktop backlog
[#21](https://github.com/tjorim/ElevenClock/issues/21) (source: #9) once recorded
there, with hosted attempts as above.

## Unchecked gates

- [ ] Default two-line clock on primary and all monitors with a **real** overlay
      (P1 NO-GO; the experimental candidate is live, so this is observed, not accepted).
- [ ] Explorer kill/restart restores tray/clock state and rejects old discovery
      results **on a real shell** (stale-result rejection is proven with scripted and
      fixture inputs; hosted lifecycle script measures survival and no accumulation).
- [ ] Hotplug, DPI changes, lock/unlock, resume and timezone/manual time on real
      displays (needs mixed-DPI hardware: [#21](https://github.com/tjorim/ElevenClock/issues/21)).
- [x] Correlate an Explorer restart with a demonstrably outstanding UIA call/object and
      record HRESULTs (Explorer itself) — **hosted only**, see "UIA lifetime evidence";
      repeating it on a real desktop shell is covered by the interactive matrix below.
- [ ] Review of production same-executable helper activation/lifetime
      ([ADR 0002](../adr/0002-same-executable-discovery-helper.md)) by a person.
- [ ] Interactive matrix: shell restart, monitor changes, auto-hide/fullscreen,
      sleep/lock, clock changes with steady-state thread/handle counts.

The [hosted lifecycle workflow](../../.github/workflows/runtime-lifecycle.yml) runs
`scripts/test-rust-lifecycle.ps1` on Server 2025 x64 and Windows 11 ARM64 (native and
emulated x64): quiet start, second launch, close-to-background, Exit, three Explorer
restarts with process/thread/handle accounting. Its development-overlay step is
`continue-on-error` *informational* evidence only.

## Known limitations

* Overlay creation failure is reported (`PlacementFailed`) but the reducer keeps its
  view of that overlay until the next Explorer loss or topology change.
* DPI or monitor changes invalidate visibility immediately, releasing the old
  cover before discovery completes. A fresh batch recreates and repaints it even
  when the physical clock rectangle is unchanged.
* The overlay does not forward clicks; clock actions, the menu, tray icon and UIA
  element are [R09](interaction.md).
* Auto-hide classification (partly off-monitor = hidden; fully on-monitor but
  different = relocated) is a documented heuristic awaiting real-shell evidence.
* Hook coverage and the 2 s poll are the only defence if an event is missed; hook
  count is exposed for diagnostics (`Notifications::hook_count`).

## Issue #34 recovery follow-up (2026-10-16)

Parked taskbars are classified as hidden even when their rectangle equals the
discovery snapshot. Each taskbar is observed independently; revealing a taskbar
whose clock could not be validated schedules fresh discovery, including a reveal
with unchanged window bounds. Offscreen clock candidates remain rejected.

The observation pass also compares the current shell's top-level taskbar handles
with the last discovery batch. This cheap native inventory uses no UI Automation
and detects a secondary taskbar created after a partial Explorer-startup batch,
even if its event was missed. Requests share the existing two-second throttle,
settle window and worker; failed discovery retains its exponential backoff.
Overlay DPI/display messages now queue topology signals as well as invalidating
pixels. Stale geometry releases the cover before the bounded helper finishes.

Deterministic tests cover parked snapshots, independent reveals on two displays
(including negative coordinates), unchanged-bounds reveal, late secondary
creation, retry backoff, and same-bounds DPI changes. Workspace tests and Clippy
passed locally on Windows, as did the native x64 release build; the startup registry tests required an unsandboxed
rerun after the sandbox denied access to their temporary test keys.

Computer Use listed only the Codex window. Opening Windows Settings first timed
out awaiting app approval; a retry reported no targetable window, and a fresh
window inventory still contained only Codex. Launching Control Panel by its
returned app ID also reported no targetable window. With full access enabled,
a later retry successfully captured and navigated Control Panel. Its Taskbar and
Navigation link opened a Settings window, but Computer Use then explicitly
returned "Computer Use was not approved to use Settings" on target activation.
The subsequent approved retry reached Settings and exercised the workspace
release build on Windows x64 26300.9711 (26H2), with two 1280x720 displays at
96 DPI in a QEMU guest. It exposed a split secondary clock with a separate bell
and no Show Desktop button. The selector now accepts that secondary layout while
still requiring its bell and complete neighbour exclusions; both clock slots
were then located. Runtime logs show primary recovery after an auto-hide reveal.
This is partial evidence, not a visual or two-display recovery acceptance pass.
Auto-hide and the primary-only preference were restored, and test instances exited.
See the [capture and run record](../windows-evidence/issue34-26300/README.md).
Independent secondary reveal, Explorer restart, physical-hardware checks,
independent hit-target review and human ADR review remain open in
[#34](https://github.com/tjorim/ElevenClock-rs/issues/34).

The [independent follow-up](../windows-evidence/issue34-26300/follow-up/README.md)
later verified secondary reveal twice and settled visual/native pointer coverage
on both displays, including neighbour exclusion. It also recorded ten failed
Explorer recovery samples and a timed-out dense UIA identity experiment. Visible,
selected, on-monitor clocks that remain unverified after a successful batch now
get four bounded settling retries (2, 4, 8, 16 seconds); unsupported layouts do
not create an endless discovery loop. This did not make the repeated Explorer
test pass, so that acceptance item and P1 remain open.
