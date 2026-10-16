# Rust dependency policy

R01 uses `windows` 0.61 for documented Win32 controls and safe boundary types,
`chrono` 0.4 with only `std` for datetime contracts, and build-only
`embed-resource` 3 for the dialog, existing icon and DPI-v2 manifest. Optional
serialization, logging and error dependencies belong to their owning issues.
No command parser, async runtime or web UI runtime is required.

Commit Cargo.lock and use --locked for all checks/builds. New dependencies must
explain product need, maintenance, feature selection and license impact.
Update in a dedicated PR targeting rust-rewrite; run the Linux and Windows
jobs and the advisory/license inventory checks. Do not raise the declared
`rust-version` merely to absorb an update; record the justification if it changes.
Transitive updates need the same checks. No audit failure is silently suppressed;
any exception needs a reviewed, dated rationale and remediation issue.

CI cargo-audit denies warnings; unavailable advisory data is a failure. License
checks fail on terms outside the explicit reviewed set or with missing original
LICENSE/COPYING/NOTICE text; CI does not upload the inventory. Regenerate it on
demand with `cargo metadata --manifest-path rust/Cargo.toml --locked --format-version 1 > dependency-metadata.json`
and `python3 scripts/rust-license-check.py dependency-metadata.json`. Review the
inventory on updates. SPDX metadata is not a substitute for shipping attribution.
The inventory script is development-only. Release packaging must retain applicable
notices and satisfy the selected terms for each dependency.

The repository LICENSE, Python code and existing icon notices are preserved.
The inherited license-file points at that existing LICENSE; it does not assert
that the README/LICENSE conflict has been resolved or rebrand the project.
R14 must establish the applicable GPL/asset obligations before distribution.
The [packaging workflow](releasing.md) now includes the original inventory and
toolchain notices in unsigned development candidates, plus matching vendored
source. Distribution remains blocked by #32 and the Windows acceptance gates.
CI uploads test evidence (screenshots, JSON, logs) and one runnable development
build (`elevenclock-dev-x64`) for testing; neither is a release package or installer.

R02 adds **development-only** `serde_json` 1.0 for the probe's structured evidence
and generation-envelope tests. It uses the maintained Serde JSON implementation
with default std support and no derive requirement; the locked version is checked
on MSRV 1.90.0 and stable. Its new lock entries are serde, serde_json, itoa and
zmij (MIT/Apache-2.0 alternatives); existing metadata dependencies are reused.
It is a dev-dependency of clock-windows and does not enter clock-app's normal
dependency graph. The Windows bindings enable documented COM, UIA, registry,
process and job APIs; no additional runtime package, command parser or async
runtime is introduced. The license check covers these developer
dependencies too.

R04 adds development-only **proptest** (locked 1.12.0) for generated formatter
validation and **Criterion** 0.7.0 (default features disabled) for parse/render
measurements. Both compile on MSRV 1.90 and stable 1.99. Neither changes the
normal app/core runtime dependency graph. Their MIT/Apache-2.0 alternatives and
transitive original notices are retained by the same inventory gate.

The notice checker recognizes only explicit historical dual-license aliases:
`MIT/Apache-2.0`, `Apache-2.0 / MIT` and `Unlicense/MIT`. They correspond to the
already reviewed OR choices; arbitrary slash expressions remain rejected.
For `rustix`/`linux-raw-sys`'s
`Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` and
`zerocopy`/`zerocopy-derive`'s `BSD-2-Clause OR Apache-2.0 OR MIT`, this project
selects the **MIT alternative**. Acceptance of these exact OR expressions does
not approve standalone BSD-2-Clause, the LLVM exception, new exceptions, or
arbitrary expressions containing MIT. Unrecognized terms still fail review.

The version-specific [supplemental notice manifest](dependency-notices/README.md)
restores anes 0.1.6's omitted original license files and recognizes r-efi 6.0.0's
bundled AUTHORS license/copyright text. Source commits and digests are retained;
missing, changed, unreadable or empty notices fail the check. These are retained
original notices, not missing-notice exemptions. The checker uses no network.
Run `python3 scripts/test-rust-license-check.py` for its positive and rejection
regressions; the dependency CI job runs them before the inventory.

R05 promotes **serde_json** 1.0 to a runtime dependency of clock-windows and adds
**serde** 1.0 with derive for the private, versioned preference encoding. These
maintained MIT/Apache-2.0 crates keep serialization behind the Windows adapter;
core remains free of persistence/schema dependencies. **tempfile** 3 is a
development-only dependency for isolated filesystem failure tests. All packages
were already locked by earlier developer dependencies; no package versions or
MSRV change. The same full license inventory applies to the new runtime graph.

R09 adds **windows-core** 0.61.2 as a direct dependency of clock-windows: the
`#[implement]` macro that builds the overlay's UI Automation provider names that crate
in its expansion. It is the version `windows` 0.61 already locks, so no package, license
or MSRV change; only the clock-windows dependency edge in `Cargo.lock` is new. The shared
`windows` dependency also enables `Win32_System_Ole` and `Win32_System_Variant`, which the
UIA provider interfaces require.
