# Rust development (R01)

The integration branch is `rust-rewrite`, created from main with the planning
contracts included. One issue per feature PR targeting that branch; do not merge
into main until the release gates pass. The Python application stays intact.
The baseline is f967218e95b6f3c15e26f840e6090a6720528e39; the reviewed plans landed
in main through #17. The old `rust-rewrite-plan` link in R01 no longer resolves.

## Toolchains and commands

Rust 2024 on current **stable** only: CI runs one toolchain, and the repository does
not pin one (no `rust-toolchain.toml`). Workspace `rust-version` (1.99, the stable release
current when it was last set) matches what CI builds with; it is not a separately
tested floor, so raise it when you move to a newer stable. Install
rustfmt and Clippy. Use `fmt --all` from the repository root so cargo-fmt finds
this virtual workspace's targets. Release builds require a native Windows host so
resources cannot be silently omitted during cross-compilation. Linux cross-target
checks are type checks only.
CI layout (`rust-ci.yml`): the Linux `core` job runs fmt, Clippy for the host and for
`x86_64-pc-windows-msvc` (the Windows-only code is type-checked and linted, not
executed) and the portable `clock-core` tests. The `windows` job only runs what needs
Windows: the full test suite including Windows-only tests, the release build with its
native resources, and the desktop smoke checks. Both jobs use a cargo cache.
Windows development requires the MSVC toolchain and
Windows SDK resource compiler; users do not need these developer tools.

```sh
cargo fmt --manifest-path rust/Cargo.toml --all --check
cargo test --manifest-path rust/Cargo.toml -p clock-core --locked
cargo test --manifest-path rust/Cargo.toml -p clock-core --doc --locked
cargo clippy --manifest-path rust/Cargo.toml -p clock-core --all-targets --locked -- -D warnings
```

On Windows, additionally run:

```sh
cargo check --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path rust/Cargo.toml --workspace --locked
cargo test --manifest-path rust/Cargo.toml --workspace --doc --locked
cargo build --manifest-path rust/Cargo.toml -p clock-app --release --target x86_64-pc-windows-msvc --locked
```

Double-click the resulting `elevenclock-rs.exe` for the native
runtime and Settings shell. It has no command suite. Closing Settings keeps the
clock running in the background; **Exit ElevenClock-rs** quits (R08, see
[its record](design/lifecycle.md)). This is a work in progress with no
release: the experimental R02 candidate places the clock overlay, and P1 is still
NO-GO (#21), so expect rough edges.
`rust-ci.yml` checks the GUI subsystem, embedded manifest, native launch/close
and forward/reverse dialog focus. Its development harness captures the actual
dialog and records runner OS/build/DPI as prototype evidence. The environment
harness uses actual Windows high contrast and Display Scale selections at
100/125/150/200%, restoring the original settings; it never injects DPI messages
or uses a private DPI setter. It is not evidence
of Explorer or interactive UI compatibility. See the [manual gate](windows-evidence/settings-foundation.md).

## Boundaries and ownership

Dependency direction: app → windows → core and app → core. Core has only
platform-independent chrono datetime values; its graph contains no Windows crate.
App/core forbid unsafe. `clock-windows` compiles native code only on Windows and
owns FFI. On Linux the app is a no-op build stub, not a supported product.

`clock-core::model` defines physical-pixel rectangles with checked signed edges,
transient generation-scoped taskbar keys, discovery snapshots, rendered text,
cadence and event/effect values. See [contracts](rewrite/dissection.md#4-target-boundaries-and-interfaces).
R02 owns snapshot validation: a Some clock rect means proven clock-only bounds;
DPI must be nonzero, clock/taskbar must fit the monitor, and adjacent controls
must be excluded. Unknown visibility/geometry must not produce overlays.

R04 implements parsing/rendering and enforces ClockText's documented bounds;
R05 owns the validated preference type and internal store. No placeholder schema
or serializer freezes that later decision. R06 owns reducer state;
PreferencesApplied carries R05 committed validated preferences. ThemeChanged
carries a resolved appearance revision;
FullscreenChanged carries a generation-scoped taskbar key and boolean. See
[R06 effect contract](design/state-and-scheduling.md) for integration and timer semantics.
Time events use one DateTime<FixedOffset> snapshot per update; it is not a stored
timezone rule. Discovery responses and all overlay effects carry ShellGeneration;
R06 must discard stale responses and destroy old-generation overlays on loss.
MonitorId/TaskbarId are process-local values, never raw handles or persistence IDs.
Changes to shared interfaces require coordinated review before dependent work.

The modal dialog manager owns HWNDs, controls, fonts and DPI layout on the calling
UI thread. The callback stores no borrowed state and never unwinds across FFI.
Shared LoadIcon resources are not destroyed. Native handles/COM pointers never
cross worker channels. Future custom owned handles need RAII, explicit disposal
order and UI-thread affinity; UIA objects belong to one bounded worker as defined
in the dissection. No blocking discovery or rendering work in callbacks.

Resource files are linked into the app binary, not an intermediate library. Use
standard-user `asInvoker` and per-monitor DPI-v2 activation. Standard dialog
processing supplies Tab/Shift+Tab, mnemonics, Escape and default-button handling.
Do not assume these work accessibly without the required desktop evidence.

Only `clock-app`'s release executable is an app artifact. Tests, examples, probes,
license inventory scripts and debug binaries are development artifacts and must
never enter an installer or support flow. R14 owns packaging and final notices.

## R02 discovery development

Portable discovery validation and the process-supervisor tests also run on Linux:

```sh
cargo test --manifest-path rust/Cargo.toml -p clock-windows --all-targets --locked
cargo clippy --manifest-path rust/Cargo.toml -p clock-windows --all-targets --locked -- -D warnings
```

The Windows-only FFI remains gated; only portable geometry/queue code is exposed
on Linux. Native/UIA probing is blocking, development-only work in an isolated
helper and must never run in application callbacks. The GUI app has no helper
integration yet. See [R02 evidence](windows-evidence/discovery.md) for developer
probe commands and the unchecked Windows gate, and the
[discovery ADR](adr/0001-taskbar-discovery.md) before any downstream use.

When the local workspace is Linux, use the
[hosted desktop workflow](../.github/workflows/runtime-lifecycle.yml) for actual
Win32/COM verification. It runs the same probe on Windows Server x64 and Windows
11 ARM64 with native ARM64 and emulated x64 binaries, retaining screenshots,
structured output, Explorer-restart observations and helper-lifetime results.
Do not treat ARM64/x64 emulation as a native Windows 11 x64 OS, count two binaries
on one OS build as two builds, or replace mixed-DPI evidence with a single display.
The evidence record lists the exact verified environments and remaining gates.

## Shared desktop acceptance backlog

Use [#21](https://github.com/tjorim/ElevenClock/issues/21) across rewrite issues
only for checks that cannot be meaningfully verified on an appropriate hosted
runner. Try hosted Windows images and feasible harness/workflow changes first.
Missing local Windows, an interactive UI requirement or an unimplemented harness
does not justify deferral. Ordinary UI automation, hit testing, screenshots,
keyboard access and controlled COM/UIA failure tests belong in hosted CI.

Each deferred entry must link its source issue/acceptance criterion, hosted
attempts or an established platform limitation, required desktop conditions,
reproduction commands, expected evidence and the specific feature/release gate.
Group tasks by source issue. Recheck capabilities/access and move a variant back
to hosted CI when feasible; do not defer the entire issue or block unrelated work.

## R03 overlay development

`clock-overlay-spike` is a separate development GUI binary, never an installed
app artifact. It opts into `clock-windows/overlay-spike`; a normal `-p clock-app`
release build does not enable experimental overlay/interaction FFI. Core models
and the R02 production-selector gate remain unchanged. Build/run parameters,
actual hosted results and the P1 no-go are in
[overlay evidence](windows-evidence/overlay-spike.md). The hosted workflow uses
real native/UIA hits, screenshots, input and shell/display transitions; positive
window flags alone do not prove opaque coverage or calendar access. #23 fixed the
R03 recovery failure (overlays are created in the taskbar's band) and settled
the calendar strategy (one deliberate Win+N); #4 closed the prototype milestone.
P1 remains NO-GO and P3 stays blocked only on the stock-x64/mixed-DPI gate in #21.

## R07 rendering development

`clock-windows::render` holds portable layout, DPI and overflow logic that tests on
Linux, plus the Windows-only DirectWrite/Direct2D `Renderer`/`ClockSurface`; the
Windows tests in `crates/clock-windows/tests/render_native.rs` need a Windows
host. `clock-windows::theme` resolves opaque palettes (high contrast first). See
the [R07 handoff](design/rendering.md) for the API, resource ownership and the
unchecked interactive gates. `ELEVENCLOCK_RS_R07_THEME` is a development-only switch
for the overlay harness, not a user setting.

## R08 runtime development

`clock-windows::runtime::Runtime` is generic over a `Platform` and fully tested on
Linux with a scripted platform; the Windows implementation is `runtime::native`.
`tests/discovery_worker.rs` (custom `main`, no libtest harness) runs real child
processes on every host and, on Windows, job-object and parent-loss tests. The app's
discovery helper is the same executable started with a private marker
([ADR 0002](adr/0002-same-executable-discovery-helper.md)); it is not a user command.

```sh
cargo test --manifest-path rust/Cargo.toml -p clock-windows --locked
```

There is no feature flag or support gate for overlays: the experimental candidate is
always used, after geometric validation of its rectangle.
Hosted lifecycle evidence: `scripts/test-rust-lifecycle.ps1` via
[runtime-lifecycle.yml](../.github/workflows/runtime-lifecycle.yml).

## R11 support and logging development

`clock-windows::support` (status wording, the diagnostics report, redaction, the
bounded Refresh wait) and `clock-windows::logging` (the 1 MiB x 3 rotating log) are
portable and tested on any host. Only the Windows build lookup and the clipboard are
Windows-only (`support/native.rs`). The report is plain text for people, with no schema
and no command that prints it; change its wording freely, but keep it free of user names,
paths and window titles (`support/tests.rs` has the golden text and the redaction cases).
Write only what a state transition means into the log, never per tick or repaint.

The real-clipboard tests overwrite the clipboard text while they run (and restore it), so
they are ignored by default; CI runs them on its Windows runner:

```sh
cargo test --manifest-path rust/Cargo.toml -p clock-windows --test clipboard_native --locked -- --ignored --test-threads=1
```

See the [R11 record](design/diagnostics.md) for behaviour, limits and the unchecked
desktop gates.

## R12 login startup development

`clock-windows::startup` holds every rule (the quoted command line, shell-style parsing,
reconciling the registry value with the running executable, repair) as plain values tested
on any host; only `startup/native.rs` (the `HKCU` Run value and the `StartupApproved` flag)
is Windows-only. The value name `ElevenClockRust` and the `--startup` marker are shared
contracts: the legacy app owns `elevenClock`, and `instance::Role` owns the marker. Startup
state is never stored in preferences; the registry is read each time. Only an explicit user
action writes, and nothing outside this app's own value and flag may be touched.

The real-registry tests use a throwaway `HKCU\Software\ElevenClock-rs-tests\...` tree that
each test removes, so they run with the normal suite. `scripts/test-rust-settings.ps1`
saves, clears and restores the current user's real `ElevenClockRust` value around its
Startup step.

See the [R12 record](design/startup.md) for the owned value, the uninstall contract
and the unchecked manual logon and standard-account gates.
