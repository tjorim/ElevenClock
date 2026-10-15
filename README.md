# ElevenClock-rs

A customizable clock for the Windows 11 taskbar, written as a native Windows app in
Rust. It replaces the taskbar clock with a formatted one on every display, and leaves
the original clock untouched underneath, so quitting or pausing always brings it back.

> **Status: work in progress, not released.** The runtime, rendering, settings shell and
> the clock/tray interaction are implemented; several checks need a real Windows desktop
> and are still open. See [`docs/windows-evidence`](docs/windows-evidence) for what has and
> has not been verified, and the [rewrite plan](docs/RUST_REWRITE_PLAN.md) for the roadmap.

## Name

ElevenClock-rs is the Rust rewrite of [ElevenClock](https://github.com/marticliment/ElevenClock)
for Windows 11, begun as a fork of it. The `-rs` suffix is the usual marker for a Rust port.
It is a separate program from the original: the Python app it replaces lives on, frozen, in
[`legacy/`](legacy), and documents written during the rewrite (the plan, the issue specs and
`docs/windows-evidence`) say "ElevenClock" for either, depending on context. The GitHub
repository keeps its current name until it is renamed.

## Repository layout

| Path | What it is |
| --- | --- |
| [`rust/`](rust) | The Cargo workspace: `clock-core` (portable logic), `clock-windows` (Win32 adapter), `clock-app` (the executable, `elevenclock-rs.exe`), `clock-overlay-spike` (development-only prototype) |
| [`docs/`](docs) | Plan, contracts, architecture decisions and per-issue Windows evidence |
| [`scripts/`](scripts) | Hosted-desktop test harnesses and the dependency-license check |
| [`assets/`](assets) | Icons used by the build |
| [`media/`](media) | Images (banner, screenshots) |
| [`legacy/`](legacy) | The original Python/Qt app, kept as a reference until the rewrite no longer needs it, then removed |
| [`.github/workflows/`](.github/workflows) | CI for the Rust workspace and the hosted Windows checks |

## Developing

Rust stable on Windows (MSVC toolchain and Windows SDK) is needed for the full build; the
portable crates also build and test on Linux. Commands and conventions are in
[`docs/RUST_CONTRIBUTING.md`](docs/RUST_CONTRIBUTING.md); in short, from the repository root:

```sh
cargo fmt --manifest-path rust/Cargo.toml --all --check
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
```

## License

The checked-in [`LICENSE`](LICENSE) is the GNU General Public License v3. The upstream README
claims Apache 2.0 while its `LICENSE` file is GPLv3; that discrepancy is unresolved and is
tracked in the rewrite plan. This project began as a fork of
[marticliment/ElevenClock](https://github.com/marticliment/ElevenClock) and is not a
clean-room implementation, so until the licensing question is settled treat the code in this
repository as derived from the original and distributed under the GPLv3, with the original
notices retained.
