# Issue #34: Windows x64 26300.9711 follow-up

Run date: 2026-10-16, approximately 18:33–18:44 UTC. This is a QEMU guest, not
physical-hardware evidence. Windows reports 26H2, build 26300.9711, Core edition,
native x64 OS and x64 app. Two extended 1280x720 displays were at 96 DPI, with
the primary at (0, 0) and secondary at (1280, 0). The OS/build differs from the
26200.9457 guest described in #34; no claim about release channel or stock-image
provenance is made from the version string alone.

## Artifacts

- `environment.json`: build/UBR, edition, guest model, exposed CPU, toolchain and
  repository HEAD. The executable includes uncommitted changes; HEAD alone does
  not identify the tested patch.
- `executable.json`: SHA-256 of the final release executable tested here.
- `auto-hide-off.jsonl`: one full sample from the repository's bounded
  `taskbar_probe` development example, with auto-hide off. Both inventories were
  complete: no UIA failures, no truncation. This retains class/automation-ID,
  parent, rectangle and timing data; it contains no localized window titles.
- `runtime-transitions.log`: application transition-log excerpt spanning the
  initial runtime patch and the subsequent secondary-selector fix.

The first runtime-only executable had SHA-256
`DA6EE8D3150D4CAD29FD07EA1B5994E2281E890B76A0E2F9EACA5A6C308C55BC`.
The final executable additionally allows the captured split secondary layout.

## Observations and resulting change

Computer Use initially failed on app approval/window discovery. A later approved
retry captured Control Panel and opened Windows Settings through its Taskbar and
Navigation link. The exact workspace executable was launched with a `process:`
app identifier, after exiting the installed copy that a plain path launch opened.

With the initial runtime patch, the primary clock was located but the secondary
was rejected even with auto-hide off. The probe explains why:

| Display | Clock class and rectangle | Bell rectangle | Show Desktop |
| --- | --- | --- | --- |
| Primary | `SystemTray.OmniButtonLeft`, (1166,672)–(1236,720) | (1240,672)–(1264,720) | (1268,672)–(1280,720) |
| Secondary | `SystemTray.OmniButtonLeft`, (2454,672)–(2524,720) | (2528,672)–(2552,720) | Absent |

Both clocks expose direct `TimeInnerTextBlock` and `DateInnerTextBlock` children.
The primary also has another `OmniButtonRight` sibling at (1134,672)–(1158,720).
The selector incorrectly required Show Desktop for a split secondary clock.
It now requires Show Desktop only on primary taskbars, retaining the separate
bell requirement for all split clocks, shell ownership/uniqueness checks,
complete neighbour inventory and geometric validation. A regression test uses
the secondary's measured coordinates and checks missing/offscreen bell,
incomplete/unmeasured inventory and primary-without-Show-Desktop rejection.

The rebuilt app's Support page located both clock slots. Its original preference
was Primary display only. Selecting All displays and applying it produced
`secondary taskbar 1: clock replaced` at 18:40:06 UTC.

Windows Settings showed taskbars enabled on all displays and auto-hide initially
off. Auto-hide was temporarily enabled through its checkbox. Both taskbars
became hidden; one transient discovery helper exit (code 4) was followed by a
successful parked-taskbar inventory with no clock rectangles. Opening Start via
Ctrl+Escape revealed the primary taskbar; the runtime logged primary replacement
again at 18:40:48 UTC, while the secondary remained parked/unverified. The earlier
runtime-only build logged the same primary recovery at 18:35:29 UTC.

These are discovery/runtime observations. They do not independently prove opaque
coverage: a primary Start-menu screenshot still showed the native time/date,
despite the replacement log. No screenshot pass or timing bound is claimed.
An attempt to move the Settings test window toward display 2 instead snapped it
on the primary display; an independent secondary reveal was not completed.

Auto-hide was restored to off, Primary display only was reapplied, and the
workspace test instance was exited. No display topology, DPI, format, theme,
startup preference or installed executable was changed.

## Validation and remaining gate

The selector tests, workspace Clippy with warnings denied, formatting check and
native x64 release build passed after the secondary-layout change. The earlier
runtime regressions and workspace tests passed as recorded in the lifecycle
document; sandbox-denied registry tests passed when rerun outside the sandbox.

**P1 remains NO-GO.** Still needed: independent visual/hit-target review of clock,
bell, Show Desktop and neighbours; a secondary auto-hide reveal and recovery;
two-display Explorer restart repetition; live DPI coverage; and the physical
sleep/resume, hotplug and mixed-DPI checks, manual time change and human ADR review
listed in #34. This guest run does not satisfy those gates.
