# R03 non-activating overlay spike

Status: implementation and hosted harness under verification. **No P1 GO**.
R02's production selector remains disabled. This separate development executable
is never linked into clock-app or shipped/installed as a user tool.

## Entry and integration

On an ephemeral Windows development desktop:

```powershell
cargo +1.90.0 build --manifest-path rust/Cargo.toml -p clock-overlay-spike --target x86_64-pc-windows-msvc --locked
rust/target/x86_64-pc-windows-msvc/debug/clock-overlay-spike.exe evidence.json 180 primary-win-n
```

Use `all` / `all-win-n` for all discovered displays. `primary` / `all` disable
keyboard fallback so unavailable UIA Invoke leaves native access exposed.
Only these developer artifacts accept arguments. No end-user settings or terminal
flow is introduced. The prototype duration is bounded to 300 seconds.

The R02 isolated discovery inventories provide candidate identity, exact build,
DPI, monitor/taskbar geometry and shell PID plus process creation time. An R03
helper independently resolves points on a four-pixel grid plus final edges to
the native clock button, and neighboring button centers to their native targets.
These are experimental desktop observations, not R02's supported-selector gate.
The helper fails closed on incomplete UIA inventories, ambiguous candidates,
missing bell/Show Desktop, overlap, changed shell or failed hit samples.
Sampling does not prove every possible neighboring hit target; visual and
interaction evidence remain required on every claimed configuration.

A separate GUI-subsystem development binary carries an embedded asInvoker,
per-monitor DPI-v2 manifest. Its UI thread owns all HWND/GDI resources. Blocking
COM work runs only in one helper, assigned to a kill-on-close job before a stdin
handshake permits native work. Only bounded JSON/value records cross the worker
boundary. The R02 supervisor is reused; five-second deadline, observed helper
exit, and fatal teardown failure precede any replacement. No HWND/COM interface
crosses a thread/process channel.

Coverage is an opaque black rectangle with `2642.2`, clipped to the candidate,
using WS_POPUP and WS_EX_TOOLWINDOW/NOACTIVATE. WM_MOUSEACTIVATE returns
MA_NOACTIVATE. Z-order is experimental: insert relative to the actual taskbar's
preceding window, without an unconditional HWND_TOPMOST setting or Explorer
parent/style mutation. It can occupy the taskbar's topmost band while shown;
25-ms native checks hide it when taskbar geometry/DPI/identity changes, when the
taskbar is hidden/sliding, or when a foreground window covers that monitor.
This polling is a measured prototype candidate, not a production lifecycle.

Coverage is removed before every discovery and click. This intentionally makes
refresh gaps visible in the spike; R08 must implement reviewed asynchronous
lifecycle/invalidations before production integration. An eight-second lease
expires coverage if no result arrives. Native checks do not establish that UIA
clock contents/layout stayed unchanged between samples.

On one click, the overlay hides before a fresh underlying native point query.
UIA Invoke is attempted once. Only S_OK + absent Invoke pattern permits the
explicit `*-win-n` experiment; provider errors or ambiguous Invoke failures never
cause a second action. Win+N is one SendInput batch, refuses held user modifiers,
and releases accepted injected key-downs after a partial batch. No click relay,
recursive overlay invocation, retry loop or Explorer mutation occurs.

## Hosted evidence

The `R03 hosted overlay evidence` workflow prepares the real desktop using R02
and builds native Server x64, native Windows 11 ARM64 and emulated x64 probes.
Its harness captures primary/all coverage, foreground/style checks, fullscreen,
calendar action count/key states, forced termination, auto-hide shown/hidden,
neighbor clicks, Explorer loss/recovery and unsupported discovery. It restores
auto-hide and closes its foreground fixture in finally. Exact source SHA,
binary SHA-256, build/UBR, architecture target, contexts and screenshots are
retained even on failure. Visual calendar/neighbor acceptance requires inspecting
captures; API success is not enough.

| Gate | Evidence/result |
| --- | --- |
| Hosted attempt | Pending workflow execution |
| Complete clock coverage / neighbors | Unchecked pending screenshot and interaction review |
| Nonactivation / Alt-Tab exclusion | Unchecked pending actual hosted run |
| Fullscreen / auto-hide | Unchecked pending actual hosted run |
| Native calendar one-shot action | Unchecked pending actual hosted run and visual review |
| Forced termination / unsupported discovery | Unchecked pending actual hosted run |
| Explorer restart | Unchecked pending actual hosted run |
| Two stock Windows 11 native-x64 builds / mixed DPI | #21; established R02 hosted-runner limitation |

## P1 decision

**NO-GO for P3/production**, pending R03 evidence and R02's complete support gate.
This does not block development of this R03 spike. See
[R02 evidence](discovery.md), [discovery ADR](../adr/0001-taskbar-discovery.md) and
[shared desktop variants #21](https://github.com/tjorim/ElevenClock/issues/21).
Do not count ARM64 x64 emulation as native x64, count the same OS build twice,
or treat a single display as mixed DPI. Controlled unsupported-discovery rejection is injected only in the development
helper via `ELEVENCLOCK_R03_REJECT_DISCOVERY`; the harness restores its prior
process value. This proves fail-closed removal while Explorer stays unchanged,
not an actual blocked provider. Unsupported geometry removes coverage;
forced termination leaves the unmodified native clock in Explorer.

## Developer checks

Results will be recorded after local and hosted verification. Relevant R01
formatting, workspace tests/doctests and warnings-denied Clippy apply to the new
spike as well. Cross-target Linux checks are type checks, not Windows evidence.
