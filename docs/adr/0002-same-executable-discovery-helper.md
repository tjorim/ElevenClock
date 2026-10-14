# ADR 0002: same-executable discovery helper for the app runtime

Date: 2026-10-14. Work item: [R08 / #9](https://github.com/tjorim/ElevenClock/issues/9).
Status: **implemented for review; production overlays stay disabled by the P1 gate**.
Amends [ADR 0001](0001-taskbar-discovery.md), which required this arrangement to be
implemented and reviewed for `clock-app` before downstream use.

## Decision

Blocking shell and UI Automation work runs in a short-lived *helper process* that is
the app's own executable started with one private marker (`--elevenclock-discovery-helper
<generation>`, parsed only by `instance::Role`). The app never calls UIA or enumerates
Explorer's children on its UI thread, and never claims a COM call can be cancelled:
the only cancellation is terminating the process and observing its exit.

Why the same executable: the embedded per-monitor-v2 manifest (rectangles are wrong
without it) and the exact build already ship; no second binary or installer entry is
needed. Why a marker rather than a mode flag in the environment: a marker is visible
in the process list and cannot be inherited by accident into unrelated children.

## Activation (what makes the marker safe)

1. The role is chosen before any single-instance or UI work, so a helper never takes
   the instance mutex, opens Settings, or touches preferences.
2. The helper does nothing until it reads one of two exact handshake lines
   (`r08-go:native`, `r08-go:uia`) from stdin. The parent writes it only **after**
   assigning the process to a kill-on-close, one-process Windows job. Parent loss
   before assignment is EOF, and EOF exits without native work. Running the marker by
   hand (it is not a user command, is not documented for users and prints nothing)
   therefore reads no handshake and exits with code 2.
3. The helper re-checks per-monitor-v2 awareness and exits otherwise (code 3).
4. Output is one JSON report on a stdout redirected to a parent-created
   `create_new` temporary file, read only after observed exit, capped at 256 KiB and
   removed on every path. No HWND, `HMONITOR` or COM pointer crosses the boundary as a
   capability; the report's `hwnd` field is a hint the UI thread revalidates (class,
   owning process) before every use.

## Lifetime rules (all covered by tests)

* One persistent worker thread, one helper at a time, one coalesced pending request.
* Deadline 5 s including serialization and exit. Expiry, a superseding generation or
  shutdown terminates the helper; the replacement starts only after exit is observed.
* Exit not observed within 1 s is fatal: the worker stops permanently, the runtime
  records `TeardownFailed`, removes every overlay and starts no further helper.
* Generation and shell identity (PID + process creation time) must still match when a
  report is delivered; stale, malformed, oversized or foreign-schema output is
  rejected as a whole.
* Process exit (any path) closes the job, which kills a surviving helper.

## Support gate

With the gate closed (the shipped configuration) the handshake selects *native-only*
discovery: no UIA, no COM, no clock rectangle in any report the parent accepts. The
`unverified-overlays` development feature selects the UIA inventory and lets the
experimental R02 candidate supply a rectangle after the same geometric validation as
R02. It must never be enabled for a release build. See `taskbar::gate`.

## Residual risks and review points

* The marker is discoverable. Its only effect without the parent's handshake is exit.
* A helper that ignores termination would be a kernel/AV problem; the design stops
  rather than accumulate, and kill-on-close bounds it by the parent's lifetime.
* Temporary files live in the per-user temp directory under a PID-prefixed name; a
  crash can leave one file (not data of value). A startup sweep is not implemented.
* A genuinely blocked UIA provider has only been exercised through a controlled
  provider (see the R08 evidence record), not through stock Explorer.
