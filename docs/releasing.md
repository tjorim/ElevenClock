# Native x64 packaging (R14)

The installer and ZIP are **unsigned development candidates**. Distribution is
blocked by [#32's provenance, asset and name review](https://github.com/tjorim/ElevenClock-rs/issues/32),
the open [R13 acceptance gates](windows-evidence/acceptance.md), and clean Windows
standard-account installation/runtime validation. A closed tracker is not evidence
that its acceptance matrix passed. Actual results are in the
[packaging evidence record](windows-evidence/packaging.md).

## Build and artifacts

On native Windows with MSVC/SDK, stable Rust, Python 3.11+ (development only) and
[Inno Setup 6.7.3](https://jrsoftware.org/isdl.php), run from the repository root:

```powershell
cargo fetch --manifest-path rust/Cargo.toml --locked
./scripts/package-rust-release.ps1
```

The output directory must be new; choose another with `-OutputDirectory dist/name`.
Use `-Python <path>` or `-Iscc <path>` if needed. The driver builds `clock-app` with
`--locked`, `x86_64-pc-windows-msvc`, a separate `rust/target/packaging` directory,
and [`-C target-feature=+crt-static`](https://doc.rust-lang.org/reference/linkage.html#static-and-dynamic-c-runtimes).
No global flags or developer build defaults change. The release profile retains PDBs.

The PR/manual `rust-release.yml` workflow uploads these development artifacts:

| Artifact | Contents |
| --- | --- |
| `*-setup.exe` | Normal per-user installer, Start menu shortcut, uninstaller |
| `*-portable.zip` | Explicit app/notices/instructions/provenance/DLL-report file allowlist |
| `*-source.zip` | Actual repository sources, source-file SHA256 inventory, Cargo.lock, vendored Cargo sources and relocatable offline configuration |
| `SHA256SUMS.txt` | Installer, both ZIPs and raw executable hashes |
| Separate symbols artifact | Matching PDB, provenance and its own checksums |
| Packaging evidence | Actual installer/lifecycle/Settings logs and environment limitations |

Keep matching source and binary artifacts together. Modified development checkouts
are recorded honestly: source includes actual tracked and nonignored untracked
working files, omitting tracked deletions. Keep unrelated nonignored files out of
a packaging checkout. The snapshot is not falsely labelled as a clean HEAD archive.
To rebuild offline, extract the source ZIP, **change to its root**, and run:

```powershell
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --manifest-path rust/Cargo.toml -p clock-app --release --target x86_64-pc-windows-msvc --locked --offline
```

MSVC/SDK and the recorded Rust toolchain remain build prerequisites. This supplies
matching source, not a byte-for-byte reproducibility guarantee. Without `.git`, the
rebuilt Support commit says `unknown`; original provenance stays in the JSON file.

`-Mode Distribution` rejects any unpassed entry in `packaging/release-gates.json`,
missing evidence, a dirty checkout or unchanged development-only notice wording.
Change gates only through reviewed source changes. The workflow has read-only
repository permissions, no release/tag/publish step, no signing identity and no
automatic product prerequisite installer. A future signed/versioned release needs
an authorized release action and consistent notices.

## Identity and ownership

New AppId: `{97911B67-52D7-4D9B-B833-29303BC540A7}`; legacy AppId:
`{D62480B8-71F1-48CE-BEEC-9D3E172C87B5}`. Install binaries under
`%LOCALAPPDATA%\Programs\ElevenClock-rs`, data under
`%LOCALAPPDATA%\ElevenClock-rs`, shortcut in the current user's Start menu.
The issue's original directory wording is superseded by its
[follow-up](https://github.com/tjorim/ElevenClock-rs/issues/15#issuecomment-6185565842).
The R12 registry value **remains `ElevenClockRust`**.

Inno uses `PrivilegesRequired=lowest` with no elevation override; packaging verifies
the executable's `asInvoker` / `uiAccess=false` manifest. Startup stays off unless
chosen in the app. Uninstall removes installed files and the shortcut, preserving
private data and unowned files; no recursive/wildcard deletion, reset tasks, HKLM
actions or legacy process termination. It removes Run and StartupApproved values
only when the Run command matches the quoted installed executable plus `--startup`
(case-insensitive, outer whitespace ignored). Other portable commands, malformed
values and legacy `elevenClock` are left alone. Updates preserve opt-in at the same
path. Exit the app before install/update/uninstall; the runtime-window check asks
the user to exit and does not terminate an overlay app.

## Runtime and licensing

The PE gate rejects non-x64/console binaries, dynamic CRT imports including CRT API
sets, unknown DLLs and delay imports requiring review. Static CRT avoids adopting
an elevated VC redistributable installer. Import inspection does not prove clean
Windows launch: COM/DirectWrite can load further system components. Test on stock
Windows 11 x64 without VC developer runtimes, Python or Qt. Any missing runtime or
admin requirement blocks distribution pending review of supported app-local or
linking alternatives. No elevated prerequisite is silently run.

[#32](https://github.com/tjorim/ElevenClock-rs/issues/32) records upstream GPLv3
since `4b8de83` (2022-01-16), the stale later Apache README sentence, the rewrite's
derivation and the copied icon. Retain GPL and original notices; no relicensing or
permission to reuse name/assets is assumed. Package attribution credits ElevenClock /
Martí Climent and Xavier Delugo, as in `legacy/README.md`. `media/` is not installed;
existing sources/assets/notices remain in the source companion. The full dependency
inventory supplies original notices (including build/test dependencies); Rust's
COPYRIGHT.html supplies toolchain notices. Hash-checked supplemental notices retain
LF bytes on Windows. The unresolved #32 decisions block distribution.

## Lifecycle and rollback

Only in a disposable account, with neither overlay app running:

```powershell
./scripts/test-rust-installer.ps1 -DisposableAccount
# On a fresh disposable account, when an earlier supported installer exists:
./scripts/test-rust-installer.ps1 -DisposableAccount -PreviousInstaller C:/artifacts/earlier-setup.exe
```

The harness refuses existing app/data/shortcut/startup state. It checks hashes,
default install, shortcut launch, real Settings/Support journey, reinstall, owned
startup removal, private/unowned/legacy preservation, quoted Unicode paths, portable
startup preservation, ZIP launch and quiet marker launch. It preserves sentinel
files intentionally, so use a fresh account per run. With an earlier installer it
checks update hashes and restoring/launching that earlier executable. Same-version
reinstall is never counted as version-update/rollback evidence. Hosted results
record administrator membership and elevation; developer images cannot prove the
clean-runtime, non-admin or actual Windows-logon cases.

User rollback: Exit through Settings/tray, uninstall in Windows Settings > Apps,
install the saved earlier supported installer and launch from Start. Check its
Support version; choose startup again in Settings if wanted. Private data remains;
if the older version cannot read it, use its native Reset to defaults, not manual
preference edits. Verify Exit reveals the Windows clock. There is **no earlier
supported Rust installer**, so this cross-version flow remains unverified. Returning
to the native clock immediately requires only Exit/uninstall: the underlying clock
was never disabled. Start legacy ElevenClock only after the Rust app has exited.
