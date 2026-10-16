# Packaging and installation evidence (R14)

Status: **local packaging and hosted installer lifecycle passed; release blocked**. No clean-VM
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

## Hosted session, 2026-10-16

[Packaging run 38893516999](https://github.com/tjorim/ElevenClock-rs/actions/runs/38893516999)
passed developer checks, compiler setup, package integrity checks, the installer
lifecycle and all three artifact uploads. It tested PR head `c16cfef6025a` through
GitHub's merge commit `e57f2ceaf4e1320aaf6b3a11665847df0269d877`.
The runner was Windows Server 2025 Datacenter x64, build **26100.33438**, 96 DPI.
The evidence reports administrator membership and an elevated token; this is not
standard-account, clean Windows 11 or absence-of-runtime evidence.

The initial run failed because Chocolatey did not provide the requested 6.7.3
compiler package. CI now downloads the official upstream Inno Setup 6.7.3 release,
checks its pinned SHA256 and installs the compiler into the runner's temporary
directory for the current user.

`windows-packaging-evidence` contains installer logs, `results.json`, real Settings
captures, `settings/context.json` and `settings/support-report.txt`. Recorded passes:
checksums; default install and Start menu Settings/Exit; full Settings/Support
harness; same-candidate reinstall; owned startup removal; preservation of private
and unowned files and the legacy startup sentinel; quoted Unicode install paths;
portable startup/approval and malformed startup preservation; portable and quiet
marker launches. Actual Windows logon and earlier-installer rollback were not run.
The retained package/source/checksum and separate symbol artifacts are linked from
the run. No release gate was changed by this hosted pass.

## Open evidence

- [x] Hosted workflow results: installer logs, hashes, installed/portable Settings
      checks, Settings captures and Support report retained in run 38893516999.
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
