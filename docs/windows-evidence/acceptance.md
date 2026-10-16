# Windows acceptance gates (R13)

Status: **started, not passed.** One partial session on one machine. The release gate
stays **open**: most of the matrix, both performance runs and the soak have not been run.
Nothing below is inferred from CI, simulation or another build.

## Environment actually used

| What | Value |
| --- | --- |
| App commit | `940634a252c1` (`rust-rewrite`, R12 merged), release build, `x86_64-pc-windows-msvc`, `--locked` |
| OS | Windows 11 Home 10.0.**26300.9711** (26H2), x64 |
| CPU | AMD Ryzen 7 5800H, 8 logical processors |
| Machine | Virtual machine with two Red Hat QXL display adapters |
| Displays | 2 × 1280×720, both 96 DPI (100%); second at (1280, 0) |
| Theme | Default light |

Only **one** Windows build was available. The issue requires two exact stock Windows 11
x64 builds; the second is **not run**.

## Observed

Driven through the real desktop (computer use), no terminal for the user-facing steps:

- Launching the release exe starts one process and the Settings window opens from the
  taskbar button (**ElevenClock-rs Settings**, four tabs: Date and time, Appearance,
  Startup, Support).
- Preset dropdown → *ISO week date* → **Apply** shows "Settings applied."; the preview
  updates (`Now: 2642.5`).
- Support → *Copy diagnostics* copies the report; build, commit and per-taskbar status are
  recorded by the app itself:

```text
App version: 0.1.0 / App commit: 940634a252c1 / Windows: 10.0.26300.9711 (26H2)
Clock replacement: Windows clock in use
1. Primary taskbar: Windows clock in use: the clock text does not fit this taskbar.
   Monitor 1280x720 at (0, 0), DPI 96. Discovery: UI Automation; clock located: yes
2. Secondary taskbar: Windows clock in use: could not verify where the Windows clock is.
   Monitor 1280x720 at (1280, 0), DPI 96. Discovery: UI Automation; clock located: no
   "This Windows version may not be supported yet"
```

## Findings

1. **Primary taskbar: the clock is replaced on 10.0.26300.9711.** The app log shows
   `primary taskbar 1: clock replaced` at launch, and after Apply of *ISO week date*
   (`%g%V.%u`) from the default state the taskbar shows `2642.5` in place of the Windows
   clock, confirmed on screen. *(An earlier version of this file said the clock was not
   replaced; that was wrong. It was read during the transient state in finding 2.)*
2. **Open, possible stuck state:** on first Apply after launch the primary taskbar logged
   `Windows clock in use: the clock text does not fit` (09:21:58 UTC) and stayed that way
   until the user reset the app's preferences (09:26:25 UTC, `clock replaced`). A repeat of
   the same Apply from the clean state did **not** reproduce it. Cause unknown; needs a
   reproduction before it is closed or filed. The Settings Apply message said
   "Settings applied." throughout.
3. **Secondary taskbar: clock not located** (`could not verify where the Windows clock is
   on this taskbar ... This Windows version may not be supported yet`; UI Automation,
   clock located: no). The native clock is correctly left alone. This is a
   support-narrowing finding for 26300.9711 on a second display until fixed. An earlier
   run (08:08 UTC) logged the secondary as "not used: this display is not selected in
   Settings", so selection also changes the outcome; the diagnostics above were taken with
   it selected.
4. **Test runs disturb a live desktop.** `cargo test --workspace` opens UI Automation
   fixture windows (`elevenclock-rs-uia-fixture`) that take focus and intercept input on
   the machine under test. Do not run it during interactive checks.

## Recovery (executed on this VM)

| Check | Result |
| --- | --- |
| Explorer restart ×3 (`Stop-Process explorer`, 12 s apart, app left running) | **Pass.** Same app PID throughout; handles 269 → 270 → 270 → 270, threads 3. After each restart the log shows `taskbar 1/2: gone`, then re-discovery. After the third the primary clock was `replaced` again. |
| Forced app kill (`Stop-Process -Force` on the app) | **Pass.** Process gone, native Windows clock `11:46 16/10/2026` visible again on the primary taskbar (screenshot). No app window left. A leaked-hook check beyond "no window, clock back" was not done. |
| Quiet start (`--startup`) | **Pass.** One process, no Settings window, primary clock replaced within 8 s. |

Observed during the restarts, not yet judged: taskbar numbering swaps between runs (the
log shows `secondary taskbar 1` / `primary taskbar 2` after the second restart), and the
secondary taskbar is never verified (finding 3). Neither left the native clock hidden.

## DPI

**Not run: the VM cannot do it as configured.** The primary display is 1280×720 and
Windows disables **Scale** there (greyed at "100% (Recommended)"). The monitor lists
modes up to 1280×1024 only, which would allow at most 125%, so 150% and 200% are
unobtainable without a larger virtual display set on the VM host. The display mode was not
changed. 125% remains possible at 1280×1024 with a layout change; not attempted.

## Performance

`scripts/measure-rust-resources.ps1` (new, development-only) samples every
`elevenclock-rs*` process and reports CPU-time delta ÷ (wall time × logical processors),
private working set (`WorkingSetPrivate`), handles and threads.

| Mode | Wall | CPU, % of machine (8 logical) | Private working set | Handles | Threads | Processes |
| --- | --- | --- | --- | --- | --- | --- |
| Date-only (`%g%V.%u`, ISO week preset), primary clock replaced, secondary not | 601.6 s | **0.0088%** (< 0.2% target) | 2.89 → 2.05 MiB, max 2.93 (< 40 MiB target) | 224 → 223 | 6 → 5 | 1 (no helper process) |
| Minutes (`%H:%M`) | not run | | | | | |
| Seconds (`%H:%M:%S`) | not run | | | | | |

Date-only passes both targets on this VM, with one process and no growth over ten
minutes. This is **one mode on one machine**; the minute and seconds runs and the
eight-hour soak are required before any performance claim. Samples are in
`native-evidence/resources/` (CSV, 10 s interval; not committed). The 30 s runs earlier in
the session only validated the script.

Timer-wake and discovery rates are not visible from outside the process; take them from
Support diagnostics when the real runs happen.

## Matrix status

Checked boxes mean actually executed here. Everything else is **not run**.

- [x] Release build at an exact commit; Settings opens from the taskbar; preset edit,
      preview, Apply; Support status and Copy diagnostics
- [ ] Second stock Windows 11 build
- [x] Clock replacing the native clock on the primary taskbar (not on the secondary,
      finding 3)
- [ ] Single display; primary swap / hotplug; negative origins (second monitor is at +1280
      only)
- [ ] Mixed DPI 100/125/150/200%
- [ ] Auto-hide taskbar; fullscreen per monitor
- [ ] Native calendar and neighbour clicks
- [ ] Narrator
- [ ] Lock/resume; time and timezone changes
- [x] Explorer restart ×3; forced app kill reveals native clock; quiet `--startup` launch
- [ ] Repeated launch/Settings cycles; invalid stored preferences; overflow
- [x] Ten minutes date-only (pass); [ ] minutes; [ ] seconds
- [ ] Eight-hour soak
- [ ] Cancel/Reset, startup choice, closing Settings and Exit as one walkthrough from the
      Start menu, and a non-administrator account
- [x] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D
      warnings`, `cargo test --workspace --locked`: all exit 0 at `940634a`

## Handoff

Do not close #14. Supported builds: **none yet**. Next: reproduce finding 2, decide
finding 3, then run the matrix on this VM at mixed DPI and on a second build, then the
three ten-minute runs and the eight-hour soak with the script above.
