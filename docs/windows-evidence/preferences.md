# R05 preference persistence handoff

## Contracts and encoding

`clock-core::Preferences` is an editable Settings draft. `validate` checks 6–24
integer pt and the R04 formatter, normalizing CRLF. `ValidatedPreferences` has
private fields and carries the compiled format. Runtime consumers borrow it;
`draft()` returns a separate editable copy. Initial preferences are primary,
9 pt, system theme and `%H:%M\n%g%V.%u`.

`clock-windows::preferences_store::PreferenceSession` loads safe defaults on
missing data or an error. `load_error()` is the recoverable Settings status.
`apply()` validates and saves before publishing the new active value. A failed
save leaves active preferences and first-run completion unchanged. The caller
retains its editable draft. `reset_draft()` only stages defaults, and does not
reset first-run state. `complete_first_run()` persists internal onboarding state;
login startup is deliberately absent and remains owned by Windows/R12.

The production store resolves `FOLDERID_LocalAppData` with
`SHGetKnownFolderPath`, then uses a dedicated `ElevenClockRust` directory. It
never consults legacy `.elevenclock` preferences, HOME, command arguments or an
environment override. The directory inherits the current user's LocalAppData
ACLs; the app is standard-user and makes no cross-user store. This is private
application storage, not a security boundary against other processes of the same
user. The filename and encoding are implementation details, not a supported
manual-edit interface.

Maintained Serde + serde_json provide a compact internal UTF-8 encoding with a
version envelope. Reads are capped at 64 KiB; typed payload validation rejects
unknown fields, missing fields, invalid enums, bounds and formats. Version 1 is
initial; no older rewrite encoding exists, and no legacy import is supported.
Future migrations must be explicit and preserve their input before commit.
Unsupported versions (including newer versions) are detected before decoding
version-specific fields, preserved and blocked from all save/recovery actions.

Damaged/invalid data stays untouched on load and ordinary Apply. Settings must
present an explicit recovery action calling `recover(draft)`: it validates the
draft, copies and flushes the original into a uniquely named preservation file,
then saves. Backups are never silently deleted. If preservation or save fails,
the old disk data and active value remain. Recovery state clears only after
success. `StoreError` distinguishes I/O, damaged data, invalid preference fields,
unsupported version and required recovery. Its Display messages omit paths.

Saves use create-new same-directory temporary files, write_all, sync_all and
close before `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`. This documented
Windows operation handles first creation and existing-target replacement on the
same volume without a truncate-in-place window. Failures remove the temporary
file best-effort. A process kill may leave an ignored temporary file; it cannot
be mistaken for committed preferences. There is no watcher, reload operation,
per-monitor override or file export. R08's single-instance coordinator must
serialize app access; this store does not claim multi-writer transactions.

The public safe storage trait supports in-memory tests; filesystem path injection
is private to unit tests. Settings controls/wiring remain R10's responsibility.

## Verification

The existing `rust-ci.yml` Windows MSRV/stable workspace test jobs execute the
real persistence tests automatically, including first create/replacement,
locked-target (delete sharing denied), read-only denied-target and injected
pre-replacement failure. These tests require no interactive desktop and are not
deferred to the manual backlog. Portable tests cover defaults, invalid stored
values, unsupported versions, corrupt/oversized data, preservation/recovery,
active rollback, reset draft and first-run persistence.

Local results and hosted Windows results are recorded below. Cross-target checks
are type checks, not actual Windows persistence evidence. The end-to-end Settings
Apply/Cancel/Reset and first-run desktop journey remains unchecked until R10/R13;
R05 exposes the contract and does not implement those controls.

Local Linux/MSRV 1.90.0 checks (2026-10-14):

- `cargo +1.90.0 fmt --manifest-path rust/Cargo.toml --all --check`: passed.
- `cargo +1.90.0 clippy --manifest-path rust/Cargo.toml -p clock-core -p clock-windows --all-targets --locked -- -D warnings`: passed.
- `cargo +1.90.0 test --manifest-path rust/Cargo.toml -p clock-core -p clock-windows --all-targets --locked`: 31 tests passed; one intentionally ignored subprocess fixture; benchmark smoke passed.
- `cargo +1.90.0 test --manifest-path rust/Cargo.toml -p clock-core -p clock-windows --doc --locked`: two doc tests passed.
- `cargo +1.90.0 check --manifest-path rust/Cargo.toml -p clock-windows --all-targets --target x86_64-pc-windows-msvc --locked`: passed (type check only).
- `cargo +1.90.0 clippy --manifest-path rust/Cargo.toml -p clock-windows --all-targets --target x86_64-pc-windows-msvc --locked -- -D warnings`: passed (type check only).
- `python3 scripts/test-rust-license-check.py`: nine passed.
- `cargo +1.90.0 metadata --manifest-path rust/Cargo.toml --locked --format-version 1` followed by `python3 scripts/rust-license-check.py`: passed with complete original-notice inventory.

- [ ] Actual Windows MSRV and stable workspace tests (CI link/result pending).
- [ ] Windows persistence review before merge.
- [ ] Interactive Settings/onboarding journey after R10 integration.
