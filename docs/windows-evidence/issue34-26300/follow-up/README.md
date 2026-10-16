# Independent desktop verification, 2026-10-16

Same QEMU x64 guest: Windows 26300.9711, two extended 1280x720 displays at 96 DPI.
Repository base: a84756e plus the settling-retry changes. `executable.json`
identifies the desktop-tested build. The later fatal-worker timer cleanup is
covered by a deterministic test; `release-after-fatal-cleanup.json` identifies
the final compiled executable. Neither artifact identifies a stock-image release
channel or physical hardware.

## Results

| Check | Observed result | Evidence / limit |
| --- | --- | --- |
| Settled visual coverage | Pass on both displays: `2642.5`, with no native time/date showing inside the clock cover | Original taskbar-only PNGs |
| Native clock pointer ownership | 247/247 points per display hit the exact overlay HWND | Four-pixel grid plus final right/bottom edges, `inspection.json` |
| Neighbour exclusion | 598/598 primary and 91/91 secondary points hit Explorer; no rectangle overlap with clock | Bell, primary Show Desktop, other visible sibling buttons |
| Independent secondary reveal | Pass twice with settled screenshots; primary stayed parked | `autohide.json`, pointer at (1920,719) |
| Explorer restart recovery | **Fail: 0/2 overlays at every recovery sample in both five-restart runs** | Eventual recovery does not satisfy prompt recovery |
| Dense independent UIA identity experiment | **Not passed: five-second helper deadline** | `native-uia-deadline.json`; its process exit code 0 is not a pass |
| Sleep/resume | Unavailable: no supported standby or hibernation state | `sleep-capabilities.txt` |
| Manual system-time change | Not performed: `whoami /priv` did not list `SeSystemtimePrivilege` | No clock change attempted |
| Physical cable hotplug / laptop plus external mixed-DPI pair | Unavailable from this guest | Requires actual hardware or host-level monitor removal |

Native pointer sampling establishes the receiving process/window. It does not
replace exact UIA identity testing at every point or authorize the selector on
other configurations.

![Primary taskbar](primary-taskbar.png)

![Secondary taskbar](secondary-taskbar.png)

## Independent reveal

The native integration harness retains auto-hide and pointer state, waits until
both taskbars park, reveals only the selected one and samples its overlay hit.
The original and restored appbar state was 0 (auto-hide off).

The first run (`autohide-initial.json`) recovered the secondary twice, in about
1.25 s and 0.13 s. Its primary attempts never revealed the taskbar itself within
20 s; the foreground screenshot covered that monitor. Those attempts remain
failures, not evidence about primary overlay recovery.

The first fast secondary capture caught a native-clock compositor frame after
pointer ownership had changed. The settled repetition waits another 350 ms
before capture, preserving the earlier hit latency separately:

| Attempt | Secondary fully shown | Overlay hit | Capture | Other taskbar shown |
| --- | --- | --- | --- | --- |
| 1 | 139 ms | 165 ms | 707 ms | 0 |
| 2 | 120 ms | 121 ms | 596 ms | 0 |

These are individual samples, not guaranteed bounds. All four clock corners were
RGB (243,243,243), matching the opaque light palette. Both settled images were
independently inspected: replacement text is visible and the bell is uncovered.

![Independent secondary reveal 1](secondary-1.png)

![Independent secondary reveal 2](secondary-2.png)

## Explorer failure and bounded retry

The lifecycle harness ran five restarts before and five after the settling-retry
change, with All displays selected and auto-hide off. All ten recovery samples
recorded zero visible overlays. Process survival, helper bounds and Exit passed;
no more than one helper existed at once. Both overlays eventually returned near
the end of each run, which does not rescue the failed samples.

`restarts.json` and `restarts-with-settling-retry.json` retain the results; the
user-scoped SID in their startup-window description is explicitly redacted. The
harness normally observes processes for ten seconds after finding the primary
taskbar; any shell startup wait is additional time. `runtime-transitions.log`
records repeated helper deadlines and temporary unverified inventories.

The coordinator now retries a successful batch with selected, visible,
on-monitor unverified clocks at most four times (2, 4, 8, 16 seconds). Parked and
unselected clocks do not start this series; complete discovery resets it and
fatal worker shutdown cancels it. **This did not make the desktop restart test
pass.** Deterministic tests cover retry recovery, bounds, eligibility and cleanup.

## Reproduction and cleanup

On an interactive disposable desktop, with the real app running on All displays:

```powershell
rust/target/debug/examples/taskbar_probe.exe 1 > native-evidence/probe.jsonl
./scripts/inspect-rust-taskbars.ps1 -ProbeCapture native-evidence/probe.jsonl -OutputDirectory native-evidence/inspection
./scripts/test-rust-autohide.ps1 -Inspection native-evidence/inspection/inspection.json -OutputDirectory native-evidence/autohide -Repeats 2 -SecondaryOnly
```

The inspection script is read-only: native queries and taskbar captures, without
UIA or input. The auto-hide test uses the existing harness's documented appbar
and virtual-desktop pointer APIs, restores both states in `finally`, and fails
if the selected overlay does not recover or the other taskbar is shown.
Screenshots still require human inspection. `probe.jsonl` is the fresh inventory
used for the independent pointer checks; the task-list button's workspace-path
AppID is redacted in both retained probe captures. Clock/tray selectors, parent
relationships and geometry are unchanged.

Primary display only was reapplied and the app exited. No installed executable,
display topology, DPI, format, theme or startup setting changed. Full workspace
tests (including registry tests), Clippy and the native release build passed;
the final clock-windows library run, including fatal-worker cleanup, passed all
212 tests.

**P1 remains NO-GO.** Explorer recovery, exact independent UIA identity, other
required Windows/DPI configurations, physical hardware and human ADR review
remain open. This supersedes the earlier missing independent secondary reveal
and settled visual/native-window coverage observations only.
