# Rust dependency policy

R01 uses `windows` 0.61 for documented Win32 controls and safe boundary types,
`chrono` 0.4 with only `std` for datetime contracts, and build-only
`embed-resource` 3 for the dialog, existing icon and DPI-v2 manifest. Optional
serialization, logging and error dependencies belong to their owning issues.
No command parser, async runtime or web UI runtime is required.

Commit Cargo.lock and use --locked for all checks/builds. New dependencies must
explain product need, maintenance, feature selection, MSRV and license impact.
Update in a dedicated PR targeting rust-rewrite; run both MSRV/current stable
Linux and Windows jobs and the advisory/license inventory checks. Do not change
the compiler floor merely to absorb an update. Record justification and update
workspace rust-version, rust-toolchain.toml, CI and docs together if it changes.
Transitive updates need the same checks. No audit failure is silently suppressed;
any exception needs a reviewed, dated rationale and remediation issue.

CI cargo-audit denies warnings; unavailable advisory data is a failure. License
checks fail on terms outside the explicit reviewed set and retain the dependency
metadata plus original LICENSE/COPYING/NOTICE files as an artifact. Review the
inventory on updates. SPDX metadata is not a substitute for shipping attribution.
The inventory script is development-only. Release packaging must retain applicable
notices and satisfy the selected terms for each dependency.

The repository LICENSE, Python code and existing icon notices are preserved.
The inherited license-file points at that existing LICENSE; it does not assert
that the README/LICENSE conflict has been resolved or rebrand the project.
R14 must establish the applicable GPL/asset obligations before distribution.
CI prototype artifacts include the repository LICENSE and this policy; they are
verification builds, not a release package or installer.
