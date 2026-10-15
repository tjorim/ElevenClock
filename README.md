# Mainspring

A customizable clock for the Windows 11 taskbar, written as a native Windows app in
Rust. It replaces the taskbar clock with a formatted one on every display, and leaves
the original clock untouched underneath, so quitting or pausing always brings it back.

> **Status: work in progress, not released.** The runtime, rendering, settings shell and
> the clock/tray interaction are implemented; several checks need a real Windows desktop
> and are still open. See [`docs/windows-evidence`](docs/windows-evidence) for what has and
> has not been verified, and the [rewrite plan](docs/RUST_REWRITE_PLAN.md) for the roadmap.

## Name

Mainspring is the new name of the Rust rewrite of ElevenClock. Documents written before the
rename (the rewrite plan, the issue specs and the files under `docs/windows-evidence`)
still say "ElevenClock"; where they mean the product, read Mainspring, and where they mean the
Python app, that app is the one in [`legacy/`](legacy). The GitHub repository keeps its
current name until it is renamed.

## Repository layout

| Path | What it is |
| --- | --- |
| [`rust/`](rust) | The Cargo workspace: `clock-core` (portable logic), `clock-windows` (Win32 adapter), `clock-app` (the executable, `mainspring.exe`), `clock-overlay-spike` (development-only prototype) |
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

Apache License 2.0, see [`LICENSE`](LICENSE). This project began as a fork of
[marticliment/ElevenClock](https://github.com/marticliment/ElevenClock); the original
copyright notice is retained. How the rewrite is licensed and attributed is tracked in
the rewrite plan and is not final.
