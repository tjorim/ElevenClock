# Packaging and installation evidence (R14)

Status: **packaging under local verification; release blocked**. No clean-VM
standard-account lifecycle or earlier supported release rollback has been claimed.
See [release instructions](../releasing.md) for build and ownership contracts.

## Local session, 2026-10-16

Source base `ab2a963` plus uncommitted R14 changes (marked modified in provenance).
Toolchain: cargo 1.99.1, native Windows MSVC/SDK, Inno Setup 6.7.3. This account has
an existing running development clock; packaging uses a separate target directory
without replacing or terminating it. This is not a clean runtime/standard-account test.

- `cargo fmt --manifest-path rust/Cargo.toml --all --check`: passed.
- `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --locked`:
  passed outside the sandbox after temporary-registry writes were denied inside it.
  Clipboard tests and the explicitly opt-in Explorer restart were not run.
- `python scripts/test-rust-license-check.py`: 9 tests passed.
- `python scripts/test-rust-release.py`: 8 tests passed (PE/runtime/distribution
  rejection, source timestamps and missing-artifact rejection).
- Locked static-CRT release build passed: x64 PE32+, GUI subsystem and matching PDB.
- Notice inventory initially rejected CRLF conversions of retained originals.
  LF checkout attributes now preserve their expected SHA256 bytes; no digest waiver.
- Full package build and Inno compilation passed. Candidate installer, portable ZIP,
  source ZIP, PDB and checksums were generated locally. Executable and installer are
  verified unsigned; executable's embedded manifest is `asInvoker` / `uiAccess=false`.
  The source ZIP includes locked vendored dependencies and original notices.
- Artifact SHA256 verification, portable ZIP allowlist and source-manifest digests
  passed using `python scripts/rust-release.py verify --output dist/candidate-4`.
  Full `cargo metadata --manifest-path rust/Cargo.toml --locked --offline
  --format-version 1` succeeded from the extracted source tree using its vendored
  configuration. This is dependency resolution evidence, not an offline rebuild.

Observed direct DLL imports:

```text
advapi32.dll, api-ms-win-core-synch-l1-2-0.dll,
api-ms-win-core-winrt-error-l1-1-0.dll, bcryptprimitives.dll, comctl32.dll,
d2d1.dll, dwrite.dll, kernel32.dll, ntdll.dll, ole32.dll, oleaut32.dll,
shell32.dll, uiautomationcore.dll, user32.dll, wtsapi32.dll
```

No direct VC/UCRT redistributable, Python or Qt imports. This does not establish
clean-machine launch, full transitive runtime dependencies or another OS build's
compatibility. No elevated prerequisite is adopted.

## Open evidence

- [ ] Hosted workflow results: installer logs, hashes, installed/portable Settings
      screenshots and Support report. No remote run dispatched from this tree yet.
- [ ] Clean stock Windows 11 x64 **non-administrator account**: record exact OS/app
      build, hashes, account groups/elevation and absence of developer runtimes;
      run the disposable lifecycle harness there.
- [ ] GUI installer without UAC/machine-wide writes, default path, current-user Start
      shortcut. Shortcut and double-click ZIP launch: no console, first-run Settings,
      startup off, no manual preferences.
- [ ] UI journey: preset/edit/preview/Apply, Cancel/Reset, Support/Copy diagnostics,
      close-to-background, second-launch focus, Exit reveals Windows clock. Retain
      screenshots/report; no simultaneous Rust and legacy overlay apps.
- [ ] UI startup on/off without UAC; actual sign-out/in launches quietly once without
      Settings/console; uninstall removes only its owned registration.
- [ ] Install/update/uninstall, quoted Unicode paths, unowned/private files, legacy
      configuration, and other portable startup/approval survival. The harness covers
      feasible hosted variants; observed standard-account results still needed.
- [ ] Missing-runtime test: no preinstalled VC developer runtime; record success or
      exact missing prerequisite. Any unavoidable admin requirement blocks adoption
      pending owner review of app-local/linking alternatives.
- [ ] Earlier **supported** installer: update then rollback restores its exact hash,
      Start launch and native-clock recovery. No earlier supported Rust installer
      exists. Check older-version handling of newer preferences through native UI.
- [ ] #32 licensing/name/asset decisions, R13 acceptance, signing/release identity.
      Candidates are unsigned; no automatic public release. Distribution remains
      blocked in `packaging/release-gates.json`.

The PR/manual workflow attempts feasible hosted lifecycle/Settings cases. Its
administrator/developer image cannot demonstrate runtime absence, genuine non-admin
membership or real logon. Preserve actual results before changing a release gate.
