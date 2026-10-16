# ADR 0002: same-executable discovery helper for the app runtime

Date: 2026-10-14. Work item: [R08 / #9](https://github.com/tjorim/ElevenClock/issues/9).
Status: **implemented for review; work in progress (P1 is still NO-GO, so overlay placement is experimental)**.
Amends [ADR 0001](0001-taskbar-discovery.md), which required this arrangement to be
implemented and reviewed for `clock-app` before downstream use.

## Decision

Blocking shell and UI Automation work runs in a short-lived *helper process* that is
the app's own executable started with one private marker (`--elevenclock-rs-discovery-helper
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

## Discovery mode

The handshake also selects the helper's discovery mode. The parent always requests
`r08-go:uia`, so the UIA inventory runs and the experimental R02 candidate may supply a
clock rectangle after the same geometric validation as R02; the parent decides, never
the helper. The `r08-go:native` mode (no UIA, no COM) remains for supervision tests whose
fixture process cannot run UIA. There is no build flag or runtime gate.

## Residual risks and review points

* The marker is discoverable. Its only effect without the parent's handshake is exit.
* A helper that ignores termination would be a kernel/AV problem; the design stops
  rather than accumulate, and kill-on-close bounds it by the parent's lifetime.
* Temporary files live in the per-user temp directory under a PID-prefixed name; a
  crash can leave one file (not data of value). A startup sweep is not implemented.
* A blocked provider is exercised through a controlled window whose UI thread does
  not pump messages. Explorer is deliberately suspended to hold a UIA call in
  flight before termination; this is not a naturally occurring Explorer hang.

## Measured COM behaviour (issue #35, 2026-10-16)

[Hosted lifecycle run 38839269796](https://github.com/tjorim/ElevenClock-rs/actions/runs/38839269796)
at PR #33 head `c0c2541139e9e51040fe38e9c34c5cc2e52963d8` passed the UIA lifetime
step on Server 2025 x64 and Windows 11 ARM64, both native and emulated x64.
The [retained logs and measurements](../windows-evidence/uia-lifetime-hosted/README.md)
separate real UIA observations from fixture/scripted generation-rejection tests.

* The production inventory against the blocked window reached the test's 4 s
  deadline in 4.015–4.018 s; one helper was spawned and its exit observed on each
  target. This supports process termination as the bound, not a COM timeout.
  The runtime's 5 s deadline and 1 s teardown limit are unchanged.
* Terminating the controlled provider during `ElementFromHandle` returned `S_OK`
  on all three targets. A successful HRESULT cannot establish provider liveness.
* Explorer calls held for 507–508 ms returned `0x8000FFFF` after termination;
  retained-object calls failed with `0x80040201`. Fresh clients recovered against
  a different shell identity. Keep generation and shell-identity validation even
  when a call succeeds; never reuse COM objects across a generation.
* All three hosted shells needed the harness's manual Explorer start after its
  45 s observation window. These results do not establish automatic shell
  restart timing or physical-desktop overlay recovery (issue #34).

No lifetime assumption or timeout needs changing from these measurements.
Explicit error-returning provider modes and malformed/stale geometry injection
remain optional, unimplemented extensions in issue #35's corrected scope.
