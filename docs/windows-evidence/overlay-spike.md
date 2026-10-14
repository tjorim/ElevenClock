# R03 non-activating overlay spike

**P1 decision: NO-GO for P3/production.** The development prototype and hosted
harness are implemented, and initial opaque coverage/native access are demonstrated.
The required Explorer-restart coverage recovery fails on the recorded desktops.
This is an actual R03 hosted result, separate from R02's unavailable native-x64
and mixed-DPI variants in [#21](https://github.com/tjorim/ElevenClock/issues/21).
No supported production selector, clock rectangle or app overlay is enabled.

## Exact hosted evidence

[R03 run 38525901613](https://github.com/tjorim/ElevenClock/actions/runs/38525901613)
executed feature commit `59049385bf0e735dc7e884eb3c7e073bac529e72`; checkout/merge
revision **`b10847f66a3d4260d970d59341dd56a97942b145`**. Each profile's retained
context records the actual source, executable SHA-256, build/UBR, native CPU and
probe target. JSON values and representative original PNGs are retained in
[R03-hosted](R03-hosted/). Artifact archives contain every original capture and
R02 desktop-preparation output. Git normalizes context JSON line endings; values
are unchanged and archives retain original bytes. Images have not been synthesized
or edited.

| Actual OS | Native CPU / prototype | Exact build + UBR | Context and complete archive |
| --- | --- | --- | --- |
| Windows Server 2025 Datacenter | x64 / native x64 | **26100.33438** | [Context](R03-hosted/server-x64/context.json), [archive](https://github.com/tjorim/ElevenClock/actions/runs/38525901613/artifacts/11875971463) |
| Windows 11 Enterprise | ARM64 / native ARM64 | **26200.9457** | [Context](R03-hosted/windows11-arm-native/context.json), [archive](https://github.com/tjorim/ElevenClock/actions/runs/38525901613/artifacts/11875204553) |
| Windows 11 Enterprise | ARM64 / emulated x64 | **26200.9457** | [Context](R03-hosted/windows11-arm-x64-emulated/context.json), [archive](https://github.com/tjorim/ElevenClock/actions/runs/38525901613/artifacts/11876365625) |

All images had one **1024×768 display**. Actual scale changed through Windows
Settings from **96 to 120 DPI (100% to 125%)**. Both Windows 11 profiles restored
96 DPI and opaque coverage; Server lost discovery after scale restoration.
The two Windows 11 jobs are one OS build/architecture with different binaries,
not two Windows 11 builds or a native Windows 11 x64 OS. Hosted development images,
watermarks, startup brokers and paging-file dialogs are not stock desktop evidence.

## Acceptance and measured limitations

| Check | Recorded result |
| --- | --- |
| Initial primary/all-display clock coverage | Pass on available single display in all three profiles; native pointer hit belongs to prototype and four opaque corners verified before saving frame |
| Adjacent controls | Clock slot `[910,720,984,768]` does not intersect neighboring button rectangles; independent Rust UIA center hits and actual quick-settings/bell/Show Desktop clicks recorded |
| Nonactivation | Pass: fixture retained foreground on display; WS_EX_TOOLWINDOW/NOACTIVATE present, WS_EX_APPWINDOW absent |
| Alt-Tab | Documented toolwindow exclusion verified through actual styles; no Alt-Tab switcher screenshot captured |
| Fullscreen | Pass: actual topmost borderless foreground fixture covered the monitor; overlay hidden |
| Auto-hide | Pass: actual supported auto-hide setting/readback, physical pointer movement, hidden capture and opaque shown capture; not simulated messages |
| UIA Invoke | **Unusable in these captures:** one successful API call, no visible notifications/calendar flyout; not accepted as calendar success |
| Deliberate Win+N | Pass: a separately selected keyboard experiment/one physical click opened native notifications/calendar UI; exactly one action and released Win/N keys recorded |
| Forced termination | Pass: no prototype/helper/overlay remained; native clock visible and native hit returned to Explorer |
| Unsupported discovery | Pass: an initial live opaque lease followed by injected rejection removed coverage; native clock remained; explicitly synthetic rejection, not a blocked COM provider |
| Explorer loss | Pass: old-generation coverage removed on actual Explorer termination |
| Explorer recovery | **Fail in all profiles:** new shell identity/discovery observed, but actual opaque visible coverage did not recover; positive IsWindowVisible/SetWindowPos is insufficient |
| 100% → 125% DPI | Pass with actual Settings selection and 120-DPI opaque capture in all profiles |
| 125% → original DPI | Pass on both Windows 11 profiles; **Server fails rediscovery** (`Overlay visible=True not observed`); no guessed rectangle accepted |
| Two stock Windows 11 native-x64 builds, two mixed-DPI displays/negative topology | Unchecked; established R02 hosted-runner limitation, owned by #21 |

The automated R03 workflow is **red intentionally on measured failed criteria**;
no failure was reclassified as successful compatibility. Server additionally stops
at the DPI restore failure. The report records all demonstrated cases rather than
claiming a universal Windows compatibility result. UIA errors/stalls and retained
provider-object validation remain R09/#9 work; process teardown does not prove
COM cancellation.

![Native ARM opaque clock-only coverage](R03-hosted/windows11-arm-native/primary-visible.png)

![Separate deliberate Win+N native UI](R03-hosted/windows11-arm-native/calendar-keyboard.png)

[UIA no-flyout capture](R03-hosted/windows11-arm-native/calendar-after-click.png),
[fullscreen](R03-hosted/windows11-arm-native/fullscreen-hidden.png),
[auto-hide hidden](R03-hosted/windows11-arm-native/autohide-hidden.png),
[auto-hide shown](R03-hosted/windows11-arm-native/autohide-shown.png),
[125% opaque coverage](R03-hosted/windows11-arm-native/dpi-125.png),
[96-DPI restored](R03-hosted/windows11-arm-native/dpi-restored.png),
[forced termination](R03-hosted/windows11-arm-native/forced-termination-native.png),
[unsupported removal](R03-hosted/windows11-arm-native/unsupported-native.png),
[quick-settings click](R03-hosted/windows11-arm-native/neighbor-1.png),
[bell click](R03-hosted/windows11-arm-native/neighbor-2.png),
[Show Desktop](R03-hosted/windows11-arm-native/neighbor-3.png),
[Explorer loss](R03-hosted/windows11-arm-native/explorer-loss.png),
and [failed recovery](R03-hosted/windows11-arm-native/explorer-recovery-failure.png)
are original captures. Screenshot inspection confirms initial coverage, native
keyboard UI and native fallback. Quick-settings capture shows its loading panel;
complete settings-panel functionality is not asserted from that single frame.

## Prototype entry and changed interfaces

The separate `clock-overlay-spike` development binary is excluded from the
installed `clock-app` and from app release artifacts. Native overlay/interaction
code requires the opt-in `clock-windows/overlay-spike` feature, enabled only by
this development package; core models and R02's production gate are unchanged.

On an ephemeral Windows MSVC/SDK development desktop:

```powershell
cargo +1.90.0 build --manifest-path rust/Cargo.toml -p clock-overlay-spike --target x86_64-pc-windows-msvc --locked
rust/target/x86_64-pc-windows-msvc/debug/clock-overlay-spike.exe evidence.json 180 primary-keyboard
```

Use `all-keyboard` for all discovered displays. `primary`/`all` test UIA only;
`primary-win-n`/`all-win-n` permit fallback only for an absent pattern.
`primary-keyboard`/`all-keyboard` select a separate deliberate keyboard strategy,
never retrying a successful/ambiguous UIA invoke on the same click.
`primary-reject` is a developer fault fixture: first successful discovery followed
by rejection. Duration is bounded to 1..300 seconds. These arguments/evidence files
are not an installed command suite, user preferences or normal support flow.

Public experimental interfaces add `ExperimentSlot`, `Update`, `overlay::run`,
portable visibility/band decisions and helper-only `prototype_interaction`.
Slots contain owned physical geometry/DPI/shell epoch values, never HWNDs or COM
objects. The new package carries its own GUI-subsystem/per-monitor DPI-v2,
asInvoker/uiAccess=false manifest; R02's example manifest behavior is unaffected.

## Measured integration strategy

R02 isolated discovery supplies candidate identity, physical geometry, DPI and
shell PID plus creation time. R03 additionally resolves a four-pixel interior grid
plus final edges independently to the native clock button, and sibling button
centers to their native controls. Missing/ambiguous clock IDs, incomplete UIA,
missing bell/Show Desktop, overlap, failed hit samples or changed native epoch
reject the experiment. These samples do not establish every possible neighboring
hit target or R02's supported-selector contract on untested configurations.

The calling UI thread owns HWND/GDI/class resources. The opaque black WS_POPUP
surface draws `2642.2` only inside the measured clock button. Toolwindow/noactivate
styles and MA_NOACTIVATE prevent overlay focus. A scalar press flag plus mouse
capture pairs press/release; cancellation or release outside the slot does not
invoke. The harness confirms a stable actual native pointer target before one
physical click, not repeated input to get a passing result.

Z-order mirrors the visible taskbar's documented topmost classification. A hidden
surface enters that classification if needed, then inserts relative to the nearest
predecessor with the same classification; an already adjacent surface preserves
order. Hidden/fullscreen coverage is hidden and demoted. Only the prototype's
HWND changes. There is no Explorer parenting/style/clock-disable operation and
no unconditional global topmost policy. This strategy **has not passed recovery**;
its metadata and API success are not substitutes for visible coverage.

A 25-ms native poll checks taskbar bounds, DPI, PID/creation time and foreground
monitor coverage. Unknown foreground, sliding/hidden taskbar, fullscreen, stale
identity or expired eight-second lease hides coverage. Before each blocking
probe the UI destroys coverage and acknowledges removal to the worker. One helper
is attached to a kill-on-close one-process job before its stdin handshake permits
native/COM work. COM interfaces stay on that helper's MTA thread. Five-second
supervision plus bounded observed teardown precedes replacement; fatal teardown,
malformed/stale/oversized envelopes or missing UI acknowledgement stops supervision.
R02's supervisor implementation and real process tests are reused.

Coverage is also removed before each interaction. A fresh native point/epoch check
cannot invoke the overlay. UIA Invoke is attempted once. The raw ABI preserves
S_OK + null as unavailable; provider errors or ambiguous invoke failure do not
permit another action. Win+N is one four-event SendInput batch, refuses held user
modifiers/N and attempts release of accepted key-downs on partial insertion.
No mouse click is replayed. GUI callbacks never perform blocking UIA discovery.

The spike deliberately has refresh gaps while rechecking independent native hits;
this is not a finished renderer/lifecycle or a production performance claim.
The source and process behavior preserve native access when the experiment exits,
is terminated or refuses discovery. Controlled rejection is synthetic; actual
provider cancellation/staleness is not claimed.

## P1 handoff and remaining work

[R02 evidence](discovery.md), [its ADR](../adr/0001-taskbar-discovery.md), R03's
recorded failed recovery, and [#21](https://github.com/tjorim/ElevenClock/issues/21)
together require **NO-GO for P3/production**. Closing R02's probe milestone #3 does
not approve production rectangles. R03 development was permitted before that GO.

The demonstrated scope is initial primary/all-on-one-display coverage on the
exact hosted images and the deliberate keyboard strategy. Explorer restart
coverage remains **#4 work**, and Server DPI restore failure is recorded here;
neither is moved to #21 as an unavailable desktop condition. UIA Invoke has no
accepted functional support claim on these captures.

Before P3, diagnose recovery placement using native styles/z-order/actual hit
and frame evidence, amend the strategy or support scope, and rerun the hosted
matrix with complete opaque recovery. Preserve nonactivation, neighbor access,
fullscreen/hidden-taskbar removal and native fallback. Combine that R03 evidence
with R02's stock-x64/mixed-DPI/selector gate; do not enable production selectors
or trade native access for apparent coverage. The current PR remains a draft
and #4 is not closed as fully accepted.

## Exact developer verification

On the tested code, local **Rust 1.90.0 and stable 1.99.0** both passed:

```sh
cargo +<toolchain> fmt --manifest-path rust/Cargo.toml --all --check
cargo +<toolchain> test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo +<toolchain> test --manifest-path rust/Cargo.toml --workspace --doc --locked
cargo +<toolchain> clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo +<toolchain> clippy --manifest-path rust/Cargo.toml --workspace --all-targets --target x86_64-pc-windows-msvc --locked -- -D warnings
```

Each all-target run passed **15 unit tests** and one core doctest separately;
two ignored subprocess fixtures are deliberately invoked by supervisor tests.
The new portable tests cover negative-coordinate visibility, fullscreen, changed
geometry/lease and promotion/demotion constraints. Cross-target Linux checks are
type/lint checks only.

[Rust foundation run 38525901610](https://github.com/tjorim/ElevenClock/actions/runs/38525901610)
checks native Windows MSRV/stable builds, formatting, warnings-denied Clippy,
unit/doctests, app release/resources/Settings tests, advisories and license notices.
[R02 run 38525901616](https://github.com/tjorim/ElevenClock/actions/runs/38525901616)
passed. R03's real native/emulated builds pass; its final GUI evidence run fails
on the measured criteria above. No mock, cross-compilation or earlier weaker
IsWindowVisible-only success is used to approve P1.
