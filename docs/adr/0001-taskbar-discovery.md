# ADR 0001: fail-closed discovery and isolated UIA experiments

Date: 2026-10-14. Work item: [R02 / #3](https://github.com/tjorim/ElevenClock/issues/3).
Status: **experimental isolation decision accepted; supported selector deferred**.

Issue #3 now tracks the completed probe/hosted-feasibility milestone. The single
[shared desktop acceptance backlog #21](https://github.com/tjorim/ElevenClock/issues/21)
owns only variants blocked by documented hosted-runner limitations. Hosted-capable
hit testing and UIA error/staleness/blocked-call work remain in #4/#9. Both forms
of evidence are required for production discovery/overlay GO. PR #20 remains the
implementation review/merge vehicle.

Windows 11 taskbars may expose the clock through native children, UI Automation,
or a provider subtree whose bounds also include notification and Show Desktop
controls. A taskbar rectangle or an accessibility button alone is insufficient
proof of a clock-only hit target. No stock Windows x64 desktop is attached to this Linux environment. Hosted
Server x64 and Windows 11 ARM64 native/emulated experiments are now recorded,
but do not satisfy the two-stock-x64-build and mixed-DPI gate.

The development probe inventories both strategies. Native observations use bounded direct class lookup as well as EnumWindows,
with physical rectangles; UIA observations use AutomationId,
ClassName, FrameworkId, ControlType, ProcessId, IsOffscreen and rectangles.
It never reads localized Name properties, invokes controls, alters Explorer,
or derives a clock slot from offsets. The recorded experimental UIA candidate is a XAML OmniButtonLeft containing
TimeInnerTextBlock and DateInnerTextBlock IDs. It fits measured sibling rectangles
on the recorded hosted desktops. Supported selectors remain empty until the full
x64/mixed-DPI and independent hit-target gate passes.

Until the evidence gate passes, `verified_clock_rect` always rejects a candidate
and every probe snapshot has `clock_rect: None`. Geometric validation checks
generation, nonzero DPI, taskbar/monitor containment, a complete adjacent-control
inventory and strict intersection with bell/Show Desktop/other controls.
Touching edges are permitted. Passing geometry does not establish identity.
The existing shared core models are unchanged; the provisional snapshot strategy
field describes the native inventory path, not a chosen supported strategy.

An in-process worker cannot promise cancellation of a blocked COM provider call.
There is no evidence here that CoCancelCall or dropping interfaces provides
bounded teardown. The alternative selected for **development experiments** is an
isolated child of the same probe executable. COM is initialized MTA, owned and
released entirely on the helper's main thread. Only owned value records are
serialized; HWNDs, HMONITORs and COM interfaces stay in the helper.

The supervisor assigns the helper to an unnamed, non-inherited Windows job with
kill-on-close and a one-process limit. A stdin handshake permits native/COM work
only after job assignment; losing the parent beforehand yields EOF. A five-second
result deadline includes discovery, release, serialization and helper exit.
After expiry the parent discards output, terminates the helper and polls for exit
for at most one second. It stops on failed teardown or failed job assignment;
it never starts replacement workers while an earlier worker may remain active.
Kill-on-close covers abnormal supervisor exit. These are polling deadlines,
not real-time scheduler guarantees or evidence that COM itself was cancelled.

There is one active generation and at most one pending request. New requests
replace the pending value and invalidate the active generation's result. Only
observed helper exit frees the active slot. The development example submits
samples sequentially; the queue tests exercise concurrent-request coalescing.
No application callback or UI thread calls this blocking probe. The GUI app has
no helper mode yet: before downstream production use, the same-executable
helper arrangement, activation/lifetime and job behavior must be implemented
and reviewed for `clock-app`, with actual Windows teardown measurements.

Native inventory is capped at 2,048 children. UIA traversal is capped at 2,048
elements and depth 64, with per-call latency and failures retained. Missing or
invalid rectangles, incomplete inventory, provider errors, stale shell identity,
changed geometry/DPI/topology, truncation, deadline or generation mismatch cannot
produce an accepted clock rectangle. Whole-sample shell identity is checked
before and after discovery using process creation time as well as PID; process
IDs alone can be reused. Requests conservatively advance generation every sample.

Alternatives rejected for now: guessed fixed slots, localized labels, Explorer
injection, sending UIA interfaces across threads, abandoning blocked threads and
spawning replacements, and interpreting a timeout as COM cancellation.

**Go/no-go: NO-GO for R02's supported discovery contract and downstream overlay
use.** The probe and portable validation can be reviewed now. Two stock Windows
11 x64 builds, mixed-DPI displays and complete selector/hit-target evidence remain
unverified. Hosted Explorer failure/recovery and injected-stall job teardown are
recorded; specific in-flight UIA stale-object and actual stalled-provider behavior
remain unverified. See the
[evidence record and reproduction procedure](../windows-evidence/discovery.md).

Hosted follow-up: the three jobs in run 38508357876 pass on native Server x64,
native Windows 11 ARM64 and emulated x64 on that same ARM64 build. They record
real discovery, screenshots, Explorer failure/recovery and Windows job teardown
with confirmed injected stalls. This validates the development isolation decision
on those environments without asserting actual COM cancellation, two Windows 11
x64 OS builds, mixed DPI or complete hit-target proof. The GUI app still needs
its own reviewed helper activation/lifetime implementation before production use.
