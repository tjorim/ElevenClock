# Supplemental dependency notices

The inventory checker normally reads original LICENSE/COPYING/NOTICE files
from Cargo's extracted packages. `manifest.json` names narrowly scoped exceptions
for upstream packaging and filename conventions. Each entry matches a package
name, exact version and exact license expression; each original notice is
verified by SHA-256. Missing files, changed license expressions and digest
mismatches fail the check. The checker never downloads fallback text in CI.

- **anes 0.1.6**: the published crate omits the upstream repository-root licenses.
  Its `.cargo_vcs_info.json` identifies commit
  `378ac7461ecac29fde9e10df7b08359d1315a9ad` (tag `v0.1.6`). The original
  LICENSE-MIT and LICENSE-APACHE from that commit are copied here, together with
  `anes/README.md`, which carries the contributor copyright declaration. Files
  are unmodified. Per-file source URLs and digests are in the manifest.
- **r-efi 6.0.0**: its published `AUTHORS` contains the original license grant,
  copyright notices and author list. The crate's recorded commit is
  `7e1b0322d31d625f81a5656096330934f9cd835d` (tag `v6.0.0`). The manifest
  explicitly recognizes and checks that bundled file; no substitute license
  text is generated or fetched. Generic AUTHORS or README files are not
  accepted as notices for other packages.

The combined CI notice artifact includes all these original texts and their
source URLs. Future dependency versions require checking their packaged notices
and updating the mapping only when necessary, against their own immutable source.
