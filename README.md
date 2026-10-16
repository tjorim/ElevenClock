# ElevenClock-rs

![ElevenClock-rs icon](assets/icon.png)

A customizable clock for the Windows 11 taskbar, written as a native Windows app in
Rust. It replaces the taskbar clock with a formatted one on every display, and leaves
the original clock untouched underneath, so quitting or pausing always brings it back.

> **Status: work in progress, not released.** The runtime, rendering, settings shell and
> the clock/tray interaction are implemented; several checks need a real Windows desktop
> and are still open. See [`docs/windows-evidence`](docs/windows-evidence) for what has and
> has not been verified, and the [rewrite plan](docs/rewrite/plan.md) for the roadmap.

## Name

ElevenClock-rs is the Rust rewrite of [ElevenClock](https://github.com/marticliment/ElevenClock)
for Windows 11, begun as a fork of it. The `-rs` suffix is the usual marker for a Rust port.
It is an independent project maintained by tjorim; affiliation with or endorsement
by the original ElevenClock project is not claimed. The Python app it replaces lives on, frozen, in
[`legacy/`](legacy), and documents written during the rewrite (the plan, the issue specs and
`docs/windows-evidence`) say "ElevenClock" for either, depending on context.

## Repository layout

| Path | What it is |
| --- | --- |
| [`rust/`](rust) | The Cargo workspace: `clock-core` (portable logic), `clock-windows` (Win32 adapter), `clock-app` (the executable, `elevenclock-rs.exe`), `clock-overlay-spike` (development-only prototype) |
| [`docs/`](docs) | Plan, contracts, architecture decisions and per-issue Windows evidence |
| [`scripts/`](scripts) | Hosted-desktop test harnesses and the dependency-license check |
| [`assets/`](assets) | Original icon source and generated Windows icons |
| [`legacy/`](legacy) | The original Python/Qt app, kept as a reference until the rewrite no longer needs it, then removed |
| [`.github/workflows/`](.github/workflows) | CI for the Rust workspace and the hosted Windows checks |

## Developing

Rust stable on Windows (MSVC toolchain and Windows SDK) is needed for the full build; the
portable crates also build and test on Linux. Commands and conventions are in
[`docs/contributing.md`](docs/contributing.md); in short, from the repository root:

```sh
cargo fmt --manifest-path rust/Cargo.toml --all --check
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked
```

## License

Native x64 installer/ZIP candidate packaging and the remaining distribution gates
are documented in [the release guide](docs/releasing.md).

The independently authored Rust implementation, its new supporting tooling and
[original application artwork](assets/README.md) are licensed under
[Apache License 2.0](LICENSE). Copyright 2026 ElevenClock-rs contributors.

The inherited Python application and upstream imagery under [`legacy/`](legacy)
retain [GPLv3](legacy/LICENSE) and their original notices. Third-party dependencies
and copied notices retain their respective licenses. The upstream README's Apache
sentence is historical and does not change the legacy code's GPLv3 license.

The [provenance record](docs/windows-evidence/licensing-provenance.md) documents
the owner's statement that no Python implementation code was used in the Rust
implementation, the technical comparison, and the Apache 2.0 decision. Product-name
review and the remaining release gates are still open.

The [name and listing review](docs/windows-evidence/product-name-and-listings.md)
records the separate package identities, draft listing copy and remaining name checks.
