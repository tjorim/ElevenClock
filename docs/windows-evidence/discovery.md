# R02 taskbar discovery evidence

Status: **NO-GO; manual gate unchecked**. Prepared 2026-10-14 on Linux x86_64.
R01 is landed on `rust-rewrite`. There is no interactive Windows desktop in this
environment. No Windows build, successful selector, screenshot, mixed-DPI result
or Explorer-restart observation is claimed from cross-compilation or Linux tests.

## Evidence inventory

| Required environment | Exact build + UBR | Successful clock-only selector | Result |
| --- | --- | --- | --- |
| Stock Windows 11 x64 build A | Not run | None verified | Untested |
| Stock Windows 11 x64 build B | Not run | None verified | Untested |
| Mixed-DPI second monitor, including negative coordinates | Not run | None verified | Untested |
| Explorer restart during outstanding UIA calls | Not run | Not applicable | Untested |
| Windows helper/job termination and abnormal parent exit | Not run | Not applicable | Untested |

All Windows combinations are currently unsupported by the new discovery code.
R01's Windows Server CI evidence does not establish stock Windows 11 support.
There are **no actual Windows output fixtures** in this change. Unit-test inputs
are synthetic geometry/protocol fixtures, not desktop evidence.

Candidate strategies:

| Strategy | Probe | Evidence required before acceptance |
| --- | --- | --- |
| Native child windows | Enumerate Shell_TrayWnd / Shell_SecondaryTrayWnd and all child class/bounds observations | Actual clock-only child class/hierarchy, monitor containment and all neighboring hit-target rectangles on each supported build |
| UI Automation | Raw-view tree inventory using locale-independent properties; no Name matching | Actual AutomationId/ClassName/FrameworkId/ControlType/hierarchy selector, complete neighboring controls, latency and stale-object results |

Neither strategy is selected for supported use. A UIA parent that contains the
clock and bell, or a taskbar-wide native child, must be rejected even if its
identifier appears promising. Current snapshots always contain a null clock rect.

## Developer reproduction on Windows

These are development commands, not an installed user workflow. Only clock-app
is a product artifact; examples and test binaries must not enter the installer.
Run from the repository root on each stock Windows 11 x64 desktop with Explorer
running, the Windows SDK/MSVC prerequisites from R01, and real mixed-DPI displays:

```powershell
cargo +1.90.0 build --manifest-path rust/Cargo.toml -p clock-windows --example taskbar_probe --locked
cargo +1.90.0 test --manifest-path rust/Cargo.toml -p clock-windows --all-targets --locked
New-Item -ItemType Directory -Force evidence-r02 | Out-Null
& .\rust\target\debug\examples\taskbar_probe.exe 20 | Out-File -Encoding utf8 evidence-r02\probe.jsonl
```

The optional sample count is bounded to 1..60. Each sample has a fresh generation
and a five-second deadline; one helper at a time is permitted. A new helper
recreates UIA objects rather than retaining stale interfaces across restarts.
During a second capture, restart Explorer through Task Manager while a sample
is outstanding and retain both error/timeout samples and successful inventories.
The probe itself never restarts or changes Explorer. In-flight stale objects can
error or stall; record the observed HRESULT and termination result rather than
claiming cancellation. Also terminate the supervisor during a sample and verify
that its job-owned helper disappears. Do these experiments on disposable test
desktops and record process counts before/after repeated stalls.

Capture original screenshots at native resolution with pointer/hit-target
observations for clock, bell, Show Desktop and other tray controls. Retain the
original capture separately from an annotated copy; annotate physical rectangle
edges and element indices from the JSON. Record actual display scale, monitor
layout, auto-hide state, taskbar options, OS edition, language, toolchain, source
commit and executable hash. Do not infer hit targets from text glyph bounds.
For each proposed selector, retain observations from both builds and both DPI
values, identify all neighboring controls and test that their measured hit targets
are excluded. Include failures with auto-hide, topology/DPI changes and restarts.

The JSONL envelope records supervisor elapsed microseconds and a result with
schema/generation/status. A completed inventory includes exact CurrentBuildNumber
and UBR read from the registry, architecture, shell PID/process creation FILETIME,
signed physical taskbar/monitor rectangles, transient IDs, GetDpiForWindow DPI,
visibility, native child class/rectangles and UIA property/call timing records.
Per-call timing covers completed/error calls; a stalled call has no returned
timing and is represented by the supervisor deadline and teardown time instead.
Native inventory saturation/invalid bounds and UIA truncation/errors are explicit.
Whole-sample native shell/topology/geometry/DPI changes invalidate discovery.
Helper output is parsed only after exit and is capped at 8 MiB for acceptance.

Observable statuses include `unsupported_no_verified_selector`, `probe_error`,
`deadline`, `helper_exit` and `stale_generation`. Fatal supervisor errors go to
stderr and exit nonzero; retain stderr and exit status too. No partial timeout
output or wrong-generation record is accepted. Empty taskbar inventories also
remain unsupported. A new selector requires the ADR and evidence gate to change;
passing the geometry validator alone cannot enable overlays.

## Verification and remaining gates

The [ADR](../adr/0001-taskbar-discovery.md) records the isolation decision and
why in-process cancellation is unproven. Linux can exercise geometry, generation,
coalescing and subprocess termination. Windows cross-target checks only verify
types; they do not execute Win32, COM, registry, job or UIA calls.

- [x] Portable validation covers negative coordinates, invalid dimensions/overflow,
  containment, unknown DPI, neighboring bell/Show Desktop overlap and generations
- [x] Bounded/coalesced request model rejects a stale active result
- [x] Linux supervisor tests verify normal exit, a confirmed stalled subprocess,
  termination/reaping and rejection of completion after a deadline
- [ ] Native Windows job/COM lifetime, timeout and repeated-stall measurements
- [ ] Two exact stock Windows 11 x64 build/UBR records with original output fixtures
- [ ] Real mixed-DPI second display and annotated clock/adjacent-hit-target captures
- [ ] Successful selectors and failures justify a supported discovery strategy
- [ ] Actual Explorer restart results and no helper/thread accumulation
- [ ] Manual R02 discovery gate and go decision for downstream production code

### Local command results

Base: `rust-rewrite` 8b02b7bc3b458440130a178a1312600153316cae. Linux x86_64;
MSRV rustc 1.90.0 (1159e78c4, 2025-09-14) and stable rustc 1.99.0
(b940084d7, 2026-09-28). Each command below passed with **both** `+1.90.0`
and `+stable` (all exit 0):

```sh
cargo +TOOLCHAIN fmt --manifest-path rust/Cargo.toml --all --check
cargo +TOOLCHAIN test --manifest-path rust/Cargo.toml -p clock-core --locked
cargo +TOOLCHAIN test --manifest-path rust/Cargo.toml -p clock-core --doc --locked
cargo +TOOLCHAIN test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo +TOOLCHAIN test --manifest-path rust/Cargo.toml --workspace --doc --locked
cargo +TOOLCHAIN clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo +TOOLCHAIN check --manifest-path rust/Cargo.toml --workspace --all-targets --target x86_64-pc-windows-msvc --locked
cargo +TOOLCHAIN clippy --manifest-path rust/Cargo.toml --workspace --all-targets --target x86_64-pc-windows-msvc --locked -- -D warnings
```

Each all-targets test run passed 10 unit tests; one ignored subprocess fixture
was explicitly invoked in child test processes by the supervisor tests. One
core doctest passed. Windows FFI tests were type-checked only, never executed.
License inventory: `cargo +1.90.0 metadata --manifest-path rust/Cargo.toml
--locked --format-version 1` followed by `python3 scripts/rust-license-check.py
<metadata.json>` passed (exit 0). The normal x64 Windows clock-app dependency tree
contains no serde_json/dev serialization packages. `git diff --check` passed.
Native Windows workspace execution/release builds and the required desktop
checks remain unrun; CI is configured to run the example's supervisor tests on
Windows as well as Linux. CI results are not claimed in this local record.

Advisory audit: cargo-audit 0.22.2, `cargo-audit audit --deny warnings` from
`rust/`, exit 0; 1,310 advisories loaded and 51 lockfile packages scanned with
no reported vulnerabilities/warnings. The upstream musl release archive was
verified against its GitHub asset SHA-256 before use.
