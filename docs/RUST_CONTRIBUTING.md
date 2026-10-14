# Rust development (R01)

The integration branch is `rust-rewrite`, created from main with the planning
contracts included. One issue per feature PR targeting that branch; do not merge
into main until the release gates pass. The Python application stays intact.
The baseline is f967218e95b6f3c15e26f840e6090a6720528e39; the reviewed plans landed
in main through #17. The old `rust-rewrite-plan` link in R01 no longer resolves.

## Toolchains and commands

Rust 2024, MSRV **1.90.0** (released 2025-09-18), with a pinned development
toolchain in `rust/rust-toolchain.toml`. This floor was selected for this workspace
and verified locally, rather than inherited from another product. CI also checks
current stable (1.99.0 at this implementation). When working from the repository
root, pass `+1.90.0` explicitly; the directory override applies inside `rust/`.
Install rustfmt and Clippy. Use `fmt --all` from the repository root: Rust
1.90 cargo-fmt otherwise reports "Failed to find targets" for this virtual
workspace when its manifest is supplied from a different directory. Release builds require a native Windows host so resources cannot be silently
omitted during cross-compilation. Linux cross-target checks are type checks only.
Windows development requires the MSVC toolchain and
Windows SDK resource compiler; users do not need these developer tools.

```sh
cargo +1.90.0 fmt --manifest-path rust/Cargo.toml --all --check
cargo +1.90.0 test --manifest-path rust/Cargo.toml -p clock-core --locked
cargo +1.90.0 test --manifest-path rust/Cargo.toml -p clock-core --doc --locked
cargo +1.90.0 clippy --manifest-path rust/Cargo.toml -p clock-core --all-targets --locked -- -D warnings
```

On Windows, additionally run:

```sh
cargo +1.90.0 check --manifest-path rust/Cargo.toml --workspace --all-targets --locked
cargo +1.90.0 clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo +1.90.0 test --manifest-path rust/Cargo.toml --workspace --locked
cargo +1.90.0 test --manifest-path rust/Cargo.toml --workspace --doc --locked
cargo +1.90.0 build --manifest-path rust/Cargo.toml -p clock-app --release --target x86_64-pc-windows-msvc --locked
```

Repeat with `+stable`. Double-click the resulting `clock-app.exe` for the native
Settings foundation. It has no command suite, persistence, overlay or background
clock yet. Closing the prototype exits; R08 adds the continuing runtime lifecycle.
`rust-ci.yml` checks the GUI subsystem, embedded manifest, native launch/close
and forward/reverse dialog focus. Its development harness captures the actual
dialog and records runner OS/build/DPI as prototype evidence. The environment
harness uses actual Windows high contrast and Display Scale selections at
100/125/150/200%, restoring the original settings; it never injects DPI messages
or uses a private DPI setter. It is not evidence
of Explorer or interactive UI compatibility. See the [manual gate](windows-evidence/R01.md).

## Boundaries and ownership

Dependency direction: app → windows → core and app → core. Core has only
platform-independent chrono datetime values; its graph contains no Windows crate.
App/core forbid unsafe. `clock-windows` compiles native code only on Windows and
owns FFI. On Linux the app is a no-op build stub, not a supported product.

`clock-core::model` defines physical-pixel rectangles with checked signed edges,
transient generation-scoped taskbar keys, discovery snapshots, rendered text,
cadence and event/effect values. See [contracts](RUST_REWRITE_DISSECTION.md#4-target-boundaries-and-interfaces).
R02 owns snapshot validation: a Some clock rect means proven clock-only bounds;
DPI must be nonzero, clock/taskbar must fit the monitor, and adjacent controls
must be excluded. Unknown visibility/geometry must not produce overlays.

R04 implements parsing/rendering and enforces ClockText's documented bounds;
R05 owns the validated preference type and internal store. No placeholder schema
or serializer freezes that later decision. R06 owns reducer state and receives
committed validated preferences separately when PreferencesApplied is delivered.
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
cargo +1.90.0 test --manifest-path rust/Cargo.toml -p clock-windows --all-targets --locked
cargo +1.90.0 clippy --manifest-path rust/Cargo.toml -p clock-windows --all-targets --locked -- -D warnings
```

The Windows-only FFI remains gated; only portable geometry/queue code is exposed
on Linux. Native/UIA probing is blocking, development-only work in an isolated
helper and must never run in application callbacks. The GUI app has no helper
integration yet. See [R02 evidence](windows-evidence/discovery.md) for developer
probe commands and the unchecked Windows gate, and the
[discovery ADR](adr/0001-taskbar-discovery.md) before any downstream use.

When the local workspace is Linux, use the
[hosted desktop workflow](../.github/workflows/taskbar-probe.yml) for actual
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
