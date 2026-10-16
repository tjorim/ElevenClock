# Packaging and installation evidence (R14)

Status: **local packaging, hosted lifecycle, and standard-account development-build
install/update/uninstall/rollback and GUI checks passed; release blocked**.
Clean-machine runtime, fresh-account first run, actual Windows logon and the
remaining ownership/compatibility variants below remain unverified.
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
- [x] Two distinct development builds: update, uninstall, restore the earlier
      executable's exact hash, launch it with retained preferences, and Exit to
      the native Windows clock. This is development-build rollback mechanics,
      not cross-version schema migration or an earlier supported-release claim.
      Repeat release-to-release migration/rollback when distinct releases exist.
- [ ] #32 licensing/name/asset decisions, R13 acceptance, signing/release identity.
      Candidates are unsigned; no automatic public release. Distribution remains
      blocked in `packaging/release-gates.json`.

The PR/manual workflow attempts feasible hosted lifecycle/Settings cases. Its
administrator/developer image cannot demonstrate runtime absence, genuine non-admin
membership or real logon. Preserve actual results before changing a release gate.

## Standard-account desktop session, 2026-10-16

Computer Use exercised the actual installer and app on Windows 11 Home 26H2 x64,
build **26300.9711**, two 1280×720 displays at 96 DPI. An out-of-sandbox read of the
desktop account token confirmed **no Administrators group membership and no
elevated token**. This machine already contains Rust/MSVC/developer runtimes and
saved preferences: it is not the required clean-machine or first-run account.

Candidate: `ElevenClock-rs-0.1.0-ab2a96366e9f-x64-dev`, Support commit
`ab2a96366e9f+modified`; installed and extracted-portable executable SHA256:
`c9ee5490339cbe21fc6ec11da22d25049e305ede1474b0c90938f280e4c26e20`.
The runtime predates subsequent CI/documentation-only commits; do not relabel this
session as testing their rebuilt executable. No legacy overlay ran alongside it.

Observed through native GUI, with no end-user terminal commands or preference edits:

- Existing development clock exited through Settings. The locally built installer
  proceeded through its GPL/information pages and Install to Finish, without UAC.
  It installed into the expected current-user Programs directory and created the
  current-user `ElevenClock-rs.lnk`; the launch-after-install checkbox defaulted off.
- Start displayed ElevenClock-rs as **Recently added**. The automation tool could
  not target StartMenuExperienceHost for clicks; actual launch instead used a
  double-click on that same Start menu shortcut in Explorer's Programs folder.
  This opened Settings from the installed executable, with no console. Direct
  selection in the Start surface remains unverified in this session.
- Existing ISO-week settings survived. Keyboard editing to `%H:%M` updated the
  preview and Apply reported success. Selecting the original ISO-week preset and
  Apply restored `%g%V.%u` before further tests.
- Reset staged default settings; Cancel discarded the draft and closed Settings
  while the app remained running. Reopening Settings retained the saved ISO-week
  preset. The title-bar Close also left the same PID running; another interactive
  launch reopened Settings in that same process.
- Startup defaulted off. The checkbox enabled startup successfully without UAC,
  then disabled it successfully. The final real HKCU Run value was absent, matching
  the original off state. This is not a sign-out/sign-in test.
- Support reported the primary clock active and the secondary clock unverified,
  matching #41. Copy diagnostics reported **Diagnostics copied to the clipboard**.
  Clipboard contents were not independently pasted/compared in this session; the
  hosted harness supplies that separate evidence.
- Exit from installed Settings terminated the app (zero remaining clock processes)
  and the primary Windows clock was visibly back at 16:39 on 16/10/2026.
- The portable ZIP was extracted into a separate folder; double-clicking its EXE
  opened Settings without UAC/console. Its process path was the extracted folder,
  not the installed directory. It retained the shared private preferences and
  exited through its own Exit button. Installed and portable copies never ran
  simultaneously.

App-window captures: [installed Settings](packaging-desktop/installed-settings.png),
[format Apply](packaging-desktop/format-applied.png),
[startup enabled](packaging-desktop/startup-enabled.png),
[Support copy result](packaging-desktop/support-copied.png), and
[portable Settings](packaging-desktop/portable-settings.png).
The [observed Support text](packaging-desktop/support-observed.txt) is an
accessibility observation, not a byte-for-byte clipboard capture.

At the end of the initial GUI walkthrough the candidate remained installed, both
clock apps were stopped, original ISO-week preferences were restored, and startup
was off. Update/uninstall/rollback had not yet been exercised; their subsequent
results are recorded below. A second development candidate (`585ad4df18cc`, executable SHA256
`f9c99ec5d9d4af5201fcf55f2f1b035b0697a3961e821fd7ddba3db5821f84b1`)
was prepared for the following lifecycle
steps; it is not necessary to wait for a published supported release to test
development-build update/rollback mechanics. No release-gate boolean was changed.

### Update between two development builds

The user clicked Install on the second candidate and the wizard reached Finish.
An independent installed-file hash read matched the prepared second executable:
`f9c99ec5d9d4af5201fcf55f2f1b035b0697a3961e821fd7ddba3db5821f84b1`.
The app then launched from the installed directory without a console, retained
the saved ISO-week preset/format, and Support showed `585ad4df18cc+modified`.
The primary clock was active; the secondary remained unverified as before.
The updated app exited through Settings before preparing uninstall testing.

[Update completed](packaging-desktop/update-completed.png) and
[updated Support](packaging-desktop/updated-support.png) capture the result.
This verifies replacement and launch across two distinct **development builds**,
both package version 0.1.0. It does not prove cross-version preference migration
or rollback. Those remain separate checks. The original ISO-week private data
was retained; its pre-uninstall SHA256 is
`9b2ba72f50c74dc947212e44b137eef9ab69a25f278dae2571231b4e0a631526`.

### Standard-account uninstall

After the updated app exited, the user approved the native uninstall confirmation.
The uninstaller reported **ElevenClock-rs was successfully removed from your
computer**, with no UAC prompt. An independent filesystem/process check confirmed
that the installed EXE, installation directory and current-user Start menu shortcut
were absent, with zero remaining clock processes. The private data directory and
`preferences.dat` remained. Its SHA256 was unchanged from the pre-uninstall value
above, establishing byte-for-byte preference preservation for this run.

[Uninstall completion](packaging-desktop/uninstall-completed.png) records the
native result. Startup was already off before this uninstall; this session does
not independently verify removal of an enabled startup registration or unowned
installation-file/legacy sentinels. Those ownership variants passed in the hosted
harness and are not relabelled as standard-account results here. The earlier
development installer is retained for rollback; restoring it is the next check.

### Completed development-build rollback

The user installed the retained earlier candidate and the wizard reached Finish.
The installed executable SHA256 returned exactly to
`c9ee5490339cbe21fc6ec11da22d25049e305ede1474b0c90938f280e4c26e20`,
distinct from the second build's hash. The Start menu shortcut was recreated.
Selecting **Open ElevenClock-rs Settings** on the completion page and Finish
launched the restored executable from the installed directory. It loaded the
saved ISO-week format, and Support confirmed `ab2a96366e9f+modified`, with the
primary clock active and the secondary still unverified.

[Rollback completion](packaging-desktop/rollback-completed.png) and
[restored Support](packaging-desktop/rollback-support.png) record the result.
Exit then terminated the restored app (zero clock processes), and the Windows
clock was visible again. The private preferences hash remained exactly
`9b2ba72f50c74dc947212e44b137eef9ab69a25f278dae2571231b4e0a631526`.

This completes the A → B → uninstall → A replacement/rollback mechanics between
two distinct development executables, both version 0.1.0. It does not claim
cross-version schema migration or support for an unreleased candidate. The absence
of an earlier public release does not prevent this development-build test; repeat
release-to-release compatibility when such versions exist. The final machine state
is the earlier candidate installed but stopped, original ISO-week preferences
retained, and startup off. No release-gate boolean was changed.
